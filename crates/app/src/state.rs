use core_agent_protocol::{
    EditorAction, Envelope, GuidedEditorAction, Message, SessionStatusKind, TextSelection,
    SubagentLifecycle, SubagentLifecycleKind,
};
use core_buffer::Buffer;
use core_diff::{ChangedFile, FileDiff};
use core_fs::{FileEvent, FileTree, FileWatcherHandle};
use core_picker::{Picker, PickerPath, SearchMatch};
use core_syntax::Highlighter;
use core_terminal::{
    TerminalCommand, TerminalLauncher, TerminalRuntime, TerminalSessionConfig, TerminalSize,
    TerminalSnapshot,
};
use core_theme::Theme;
use ratatui::layout::Rect;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;
use ui_tui::terminal_view::TerminalPaneRender;

use crate::agent_bridge::{self, AgentBridgeEvent, AgentBridgeHandle};
use crate::workspace::{FocusTarget, SplitAxis, TerminalWorkspaceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Picker,
    Help,
}

#[allow(dead_code)]
pub enum ActivePicker {
    File(Picker<PickerPath>),
    ChangedFiles(Picker<ChangedFile>),
    GrepResults(Picker<SearchMatch>),
    TouchedFiles(Picker<TouchedFile>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchedFile {
    pub path: PathBuf,
    pub display_path: String,
    pub conflict: bool,
}

impl fmt::Display for TouchedFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let marker = if self.conflict { "!" } else { "~" };
        write!(f, "[{marker}] {}", self.display_path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChangeOutcome {
    Touched(String),
    Reloaded(String),
    Conflict(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSubagentState {
    pub id: String,
    pub label: String,
    pub kind: SubagentLifecycleKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSessionState {
    pub session_id: String,
    pub runtime_name: Option<String>,
    pub status_kind: Option<SessionStatusKind>,
    pub detail: Option<String>,
    pub disconnected: bool,
    pub subagents: Vec<AgentSubagentState>,
}

impl AgentSessionState {
    fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            runtime_name: None,
            status_kind: None,
            detail: None,
            disconnected: false,
            subagents: Vec::new(),
        }
    }

    fn status_label(&self) -> &'static str {
        if self.disconnected {
            "disconnected"
        } else {
            match self.status_kind {
                Some(SessionStatusKind::Starting) => "starting",
                Some(SessionStatusKind::Running) => "running",
                Some(SessionStatusKind::Completed) => "completed",
                Some(SessionStatusKind::Failed) => "failed",
                None => "attached",
            }
        }
    }

    fn active_subagent_labels(&self) -> Vec<&str> {
        self.subagents
            .iter()
            .filter(|subagent| matches!(subagent.kind, SubagentLifecycleKind::Started))
            .map(|subagent| subagent.label.as_str())
            .collect()
    }

    fn chrome_status(&self) -> String {
        let mut parts = vec![self.status_label().to_string()];
        let active_subagents = self.active_subagent_labels();
        if !active_subagents.is_empty() {
            parts.push(format!("{} sub", active_subagents.len()));
        }
        parts.join(" · ")
    }

    fn info_summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(runtime_name) = self.runtime_name.as_deref() {
            parts.push(runtime_name.to_string());
        }
        parts.push(self.status_label().to_string());
        if let Some(detail) = self.detail.as_deref().filter(|detail| !detail.is_empty()) {
            parts.push(detail.to_string());
        }
        let active_subagents = self.active_subagent_labels();
        if !active_subagents.is_empty() {
            parts.push(format!("sub: {}", active_subagents.join(", ")));
        }
        parts.join(" · ")
    }

    fn record_subagent(&mut self, lifecycle: &SubagentLifecycle) {
        let state = AgentSubagentState {
            id: lifecycle.subagent_id.to_string(),
            label: lifecycle.label.clone(),
            kind: lifecycle.kind.clone(),
        };
        if let Some(existing) = self
            .subagents
            .iter_mut()
            .find(|subagent| subagent.id == state.id)
        {
            *existing = state;
        } else {
            self.subagents.push(state);
        }
    }
}

pub struct AppState {
    pub buffers: Vec<Buffer>,
    pub active_buffer: usize,
    pub preview_buffer: Option<usize>,
    pub split_buffers: Option<(usize, usize)>,
    pub file_tree: FileTree,
    pub sidebar_visible: bool,
    pub sidebar_focused: bool,
    pub theme: Theme,
    pub highlighters: HashMap<usize, Highlighter>,
    pub mode: AppMode,
    pub picker: Option<ActivePicker>,
    pub diff_mode: bool,
    pub diffs: HashMap<PathBuf, FileDiff>,
    #[allow(dead_code)]
    pub changed_files: Vec<ChangedFile>,
    pub touched_files: Vec<TouchedFile>,
    pub touched_paths: HashSet<PathBuf>,
    pub external_conflicts: HashSet<PathBuf>,
    pub help_visible: bool,
    pub status_message: Option<(String, Instant)>,
    pub command_input: String,
    pub last_search: String,
    pub editing: bool,
    pub wrap_lines: bool,
    pub viewport_height: usize,
    pub close_dirty_confirm: bool,
    pub quit: bool,
    pub root_dir: PathBuf,
    #[allow(dead_code)]
    pub root_terminal_ratio_percent: u16,
    #[allow(dead_code)]
    pub focus_target: FocusTarget,
    #[allow(dead_code)]
    pub terminal_workspace: TerminalWorkspaceState,
    pub terminal_runtime: TerminalRuntime,
    pub agent_bridge: Option<AgentBridgeHandle>,
    pub agent_events: Option<Receiver<AgentBridgeEvent>>,
    pub agent_session_bindings: HashMap<String, u64>,
    pub agent_sessions: HashMap<String, AgentSessionState>,
    // File watching — auto-reload when agents change files on disk
    #[allow(dead_code)]
    file_watcher: Option<FileWatcherHandle>,
    pub file_events: Option<Receiver<FileEvent>>,
}

impl AppState {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;

        let normalized_paths: Vec<PathBuf> = if paths.is_empty() {
            vec![cwd.clone()]
        } else {
            paths
                .iter()
                .map(|path| {
                    if path.is_absolute() {
                        path.clone()
                    } else {
                        cwd.join(path)
                    }
                })
                .collect()
        };

        let mut roots = Vec::new();
        let mut initial_files = Vec::new();
        for path in normalized_paths {
            if path.is_dir() {
                roots.push(path);
            } else {
                initial_files.push(path.clone());
                roots.push(path.parent().unwrap_or(&cwd).to_path_buf());
            }
        }
        roots.sort();
        roots.dedup();

        let root_dir = common_ancestor(&roots).unwrap_or_else(|| cwd.clone());
        let file_tree = FileTree::build_multi(&roots, &root_dir)?;

        let mut buffers = Vec::new();
        let mut highlighters = HashMap::new();

        for file_path in initial_files {
            let buf = Buffer::from_file(&file_path)?;
            let lang = buf.language.clone();
            buffers.push(buf);
            let idx = buffers.len() - 1;
            if let Some(hl) = Highlighter::new(&lang) {
                highlighters.insert(idx, hl);
            }
        }

        // Always have at least one buffer
        if buffers.is_empty() {
            buffers.push(Buffer::from_string(""));
        }

        // Parse all buffers for syntax highlighting
        for (idx, hl) in highlighters.iter_mut() {
            let content = buffers[*idx].content();
            hl.parse(&content);
        }

        if let Some(path) = buffers.first().and_then(|buffer| buffer.path.as_deref()) {
            let mut file_tree = file_tree;
            file_tree.reveal_path(path);

            // Load changed files for diff
            let changed_files = core_diff::changed_files(&root_dir).unwrap_or_default();

            // Start file watcher — watches for changes made by agents/editors
            let (tx, rx) = std::sync::mpsc::channel();
            let watcher = core_fs::watch_directory(&root_dir, tx).ok();
            let terminal_workspace = TerminalWorkspaceState::new(root_dir.clone());
            let mut terminal_runtime = TerminalRuntime::new();
            ensure_runtime_matches_workspace(
                &terminal_workspace,
                &mut terminal_runtime,
                &root_dir,
            )?;
            let (agent_bridge, agent_events) = match agent_bridge::start_agent_bridge(root_dir.clone()) {
                Ok((handle, rx)) => (Some(handle), Some(rx)),
                Err(_) => (None, None),
            };

            return Ok(Self {
                buffers,
                active_buffer: 0,
                preview_buffer: None,
                split_buffers: None,
                file_tree,
                sidebar_visible: true,
                sidebar_focused: false,
                theme: Theme::dark_plus(),
                highlighters,
                mode: AppMode::Normal,
                picker: None,
                diff_mode: false,
                diffs: HashMap::new(),
                changed_files,
                touched_files: Vec::new(),
                touched_paths: HashSet::new(),
                external_conflicts: HashSet::new(),
                help_visible: false,
                status_message: None,
                command_input: String::new(),
                last_search: String::new(),
                editing: false,
                wrap_lines: false,
                viewport_height: 0,
                close_dirty_confirm: false,
                quit: false,
                root_dir,
                root_terminal_ratio_percent: 42,
                focus_target: FocusTarget::Editor,
                terminal_workspace,
                terminal_runtime,
                agent_bridge,
                agent_events,
                agent_session_bindings: HashMap::new(),
                agent_sessions: HashMap::new(),
                file_watcher: watcher,
                file_events: Some(rx),
            });
        }

        // Load changed files for diff
        let changed_files = core_diff::changed_files(&root_dir).unwrap_or_default();

        // Start file watcher — watches for changes made by agents/editors
        let (tx, rx) = std::sync::mpsc::channel();
        let watcher = core_fs::watch_directory(&root_dir, tx).ok();
        let terminal_workspace = TerminalWorkspaceState::new(root_dir.clone());
        let mut terminal_runtime = TerminalRuntime::new();
        ensure_runtime_matches_workspace(&terminal_workspace, &mut terminal_runtime, &root_dir)?;
        let (agent_bridge, agent_events) = match agent_bridge::start_agent_bridge(root_dir.clone()) {
            Ok((handle, rx)) => (Some(handle), Some(rx)),
            Err(_) => (None, None),
        };

        Ok(Self {
            buffers,
            active_buffer: 0,
            preview_buffer: None,
            split_buffers: None,
            file_tree,
            sidebar_visible: true,
            sidebar_focused: false,
            theme: Theme::dark_plus(),
            highlighters,
            mode: AppMode::Normal,
            picker: None,
            diff_mode: false,
            diffs: HashMap::new(),
            changed_files,
            touched_files: Vec::new(),
            touched_paths: HashSet::new(),
            external_conflicts: HashSet::new(),
            help_visible: false,
            status_message: None,
            command_input: String::new(),
            last_search: String::new(),
            editing: false,
            wrap_lines: false,
            viewport_height: 0,
            close_dirty_confirm: false,
            quit: false,
            root_dir,
            root_terminal_ratio_percent: 42,
            focus_target: FocusTarget::Editor,
            terminal_workspace,
            terminal_runtime,
            agent_bridge,
            agent_events,
            agent_session_bindings: HashMap::new(),
            agent_sessions: HashMap::new(),
            file_watcher: watcher,
            file_events: Some(rx),
        })
    }

    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<()> {
        self.exit_split();
        self.pin_preview_buffer();
        self.open_file_with_index(path).map(|_| ())
    }

    pub fn open_file_with_index(&mut self, path: &Path) -> anyhow::Result<usize> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        // Check if already open
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path.as_path()) {
                self.active_buffer = i;
                self.file_tree.reveal_path(&path);
                return Ok(i);
            }
        }

        let buf = Buffer::from_file(&path)?;
        let lang = buf.language.clone();
        self.buffers.push(buf);
        let idx = self.buffers.len() - 1;
        self.active_buffer = idx;

        if let Some(mut hl) = Highlighter::new(&lang) {
            let content = self.buffers[idx].content();
            hl.parse(&content);
            self.highlighters.insert(idx, hl);
        }

        // Reveal file in sidebar — expand parent folders and select it
        self.file_tree.reveal_path(&path);

        Ok(idx)
    }

    pub fn preview_file(&mut self, path: &Path) -> anyhow::Result<usize> {
        self.exit_split();
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Some((idx, _)) = self
            .buffers
            .iter()
            .enumerate()
            .find(|(_, buf)| buf.path.as_deref() == Some(path.as_path()))
        {
            self.active_buffer = idx;
            self.file_tree.reveal_path(&path);
            return Ok(idx);
        }

        let preview_idx = self
            .preview_buffer
            .filter(|idx| self.can_replace_preview_buffer(*idx))
            .or_else(|| self.scratch_buffer_index());

        let idx = if let Some(idx) = preview_idx {
            self.replace_buffer_with_file(idx, &path)?;
            idx
        } else {
            self.open_file_with_index(&path)?
        };

        self.preview_buffer = Some(idx);
        self.active_buffer = idx;
        self.file_tree.reveal_path(&path);
        Ok(idx)
    }

    pub fn pin_preview_buffer(&mut self) {
        self.preview_buffer = None;
    }

    pub fn close_active_tab(&mut self) {
        let removed = self.active_buffer;
        if self.buffers.len() <= 1 {
            self.buffers[0] = Buffer::from_string("");
            self.highlighters.remove(&0);
            self.active_buffer = 0;
            self.preview_buffer = None;
            self.close_dirty_confirm = false;
            return;
        }

        self.highlighters.remove(&self.active_buffer);
        self.buffers.remove(self.active_buffer);

        let mut new_highlighters = HashMap::new();
        for (idx, hl) in self.highlighters.drain() {
            let new_idx = if idx > self.active_buffer {
                idx - 1
            } else {
                idx
            };
            new_highlighters.insert(new_idx, hl);
        }
        self.highlighters = new_highlighters;

        if self.active_buffer >= self.buffers.len() {
            self.active_buffer = self.buffers.len() - 1;
        }
        self.preview_buffer = remap_buffer_index(self.preview_buffer, removed);
        self.close_dirty_confirm = false;
        if let Some((left, right)) = self.split_buffers {
            self.split_buffers = remap_split_indices(left, right, removed);
        }
    }

    #[allow(dead_code)]
    pub fn reparse_current_buffer(&mut self) {
        let idx = self.active_buffer;
        let content = self.buffers[idx].content();
        if let Some(hl) = self.highlighters.get_mut(&idx) {
            hl.parse(&content);
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }

    #[allow(dead_code)]
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.active_buffer]
    }

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active_buffer]
    }

    pub fn current_buffer_has_external_conflict(&self) -> bool {
        self.current_buffer()
            .path
            .as_ref()
            .map(|path| self.external_conflicts.contains(&self.normalize_workspace_path(path)))
            .unwrap_or(false)
    }

    pub fn clear_external_conflict_for_current_buffer(&mut self) {
        if let Some(path) = self.current_buffer().path.clone() {
            let canon = self.normalize_workspace_path(&path);
            self.external_conflicts.remove(&canon);
            self.update_touched_conflict(&canon, false);
        }
    }

    pub fn force_reload_current_buffer(&mut self) -> anyhow::Result<bool> {
        let Some(path) = self.current_buffer().path.clone() else {
            anyhow::bail!("Current buffer has no file path");
        };
        let canon = self.normalize_workspace_path(&path);
        let reloaded = self.reload_buffer_from_disk(self.active_buffer, &canon)?;
        self.external_conflicts.remove(&canon);
        self.update_touched_conflict(&canon, false);
        Ok(reloaded)
    }

    pub fn compute_diff_for_current(&mut self) {
        let buf = &self.buffers[self.active_buffer];
        if let Some(ref path) = buf.path.clone() {
            self.compute_diff_for_path(path);
        }
    }

    /// Record and react to external file changes detected by the watcher.
    pub fn handle_external_change(&mut self, path: &Path) -> ExternalChangeOutcome {
        let canon = self.normalize_workspace_path(path);
        self.record_touched_file(&canon);

        let idx = self.buffers.iter().position(|buf| {
            buf.path
                .as_ref()
                .is_some_and(|buffer_path| self.normalize_workspace_path(buffer_path) == canon)
        });

        let Some(idx) = idx else {
            return ExternalChangeOutcome::Touched(display_path(&self.root_dir, &canon));
        };

        if self.buffers[idx].dirty {
            self.external_conflicts.insert(canon.clone());
            self.update_touched_conflict(&canon, true);
            return ExternalChangeOutcome::Conflict(self.buffers[idx].file_name());
        }

        self.external_conflicts.remove(&canon);
        self.update_touched_conflict(&canon, false);
        match self.reload_buffer_from_disk(idx, &canon) {
            Ok(true) => ExternalChangeOutcome::Reloaded(self.buffers[idx].file_name()),
            Ok(false) | Err(_) => ExternalChangeOutcome::Touched(self.buffers[idx].file_name()),
        }
    }

    pub fn compute_diff_for_path(&mut self, path: &Path) {
        if let Ok(rel_path) = path.strip_prefix(&self.root_dir) {
            let rel_str = rel_path.to_string_lossy().to_string();
            let old_content =
                core_diff::git_show_head(&self.root_dir, &rel_str).unwrap_or_default();
            let new_content = self
                .buffers
                .iter()
                .find(|buf| buf.path.as_deref() == Some(path))
                .map(|buf| buf.content())
                .unwrap_or_else(|| String::new());
            let diff = FileDiff::compute(&old_content, &new_content, &rel_str);
            self.diffs.insert(path.to_path_buf(), diff);
        }
    }

    pub fn refresh_workspace(&mut self) {
        self.file_tree.refresh();
        self.changed_files = core_diff::changed_files(&self.root_dir).unwrap_or_default();
    }

    pub fn enter_split(&mut self, left: usize, right: usize) {
        self.pin_preview_buffer();
        self.split_buffers = Some((left, right));
        self.active_buffer = left;
    }

    pub fn exit_split(&mut self) {
        self.split_buffers = None;
    }

    pub fn toggle_split_focus(&mut self) {
        if let Some((left, right)) = self.split_buffers {
            self.active_buffer = if self.active_buffer == left {
                right
            } else {
                left
            };
        }
    }

    #[allow(dead_code)]
    pub fn focus_terminal(&mut self) {
        self.focus_target = FocusTarget::TerminalPane(self.terminal_workspace.active_pane_id());
        self.sidebar_focused = false;
    }

    #[allow(dead_code)]
    pub fn focus_editor(&mut self) {
        self.focus_target = FocusTarget::Editor;
        self.sidebar_focused = false;
    }

    #[allow(dead_code)]
    pub fn focus_sidebar(&mut self) {
        self.focus_target = FocusTarget::Sidebar;
        self.sidebar_focused = true;
    }

    #[allow(dead_code)]
    pub fn focus_command_bar(&mut self) {
        self.focus_target = FocusTarget::CommandBar;
        self.sidebar_focused = false;
    }

    #[allow(dead_code)]
    pub fn relaunch_active_terminal(&mut self, launcher: TerminalLauncher) -> u64 {
        let active_pane_id = self.terminal_workspace.active_pane_id();
        self.clear_agent_state_for_pane(active_pane_id);
        let pane_id = self.terminal_workspace.relaunch_active(launcher.clone());
        let Some(pane) = self.terminal_workspace.pane(pane_id) else {
            self.set_status("Terminal pane disappeared");
            return pane_id;
        };
        if let Err(error) = self.terminal_runtime.relaunch_session_with_config(
            pane_id,
            TerminalSessionConfig::new(launcher, pane.cwd.clone())
                .with_size(self.active_terminal_size())
                .with_scrollback_limit(pane.scrollback_limit),
        ) {
            self.set_status(&format!("Terminal relaunch failed: {error}"));
        }
        self.focus_target = FocusTarget::TerminalPane(pane_id);
        pane_id
    }

    pub fn launch_edit_agent_in_active_terminal(&mut self) -> anyhow::Result<u64> {
        let Some(bridge) = self.agent_bridge.as_ref() else {
            anyhow::bail!("Agent bridge is unavailable");
        };
        let pane_id = self.terminal_workspace.active_pane_id();
        let session_id = format!("pane-{pane_id}");
        self.agent_session_bindings
            .insert(session_id.clone(), pane_id);
        let launcher = TerminalLauncher::Custom(TerminalCommand::new(
            "EditAgent",
            "cargo",
            [
                "run".to_string(),
                "-p".to_string(),
                "edit-agent".to_string(),
                "--".to_string(),
                "run".to_string(),
                "--bridge".to_string(),
                bridge.socket_path().display().to_string(),
                "--session-id".to_string(),
                session_id,
                "--cwd".to_string(),
                self.root_dir.display().to_string(),
            ],
        ));
        Ok(self.relaunch_active_terminal(launcher))
    }

    pub fn process_agent_bridge_event(&mut self, event: AgentBridgeEvent) {
        match event {
            AgentBridgeEvent::Message(envelope) => self.process_agent_message(envelope),
            AgentBridgeEvent::Disconnected { session_id } => {
                if let Some(session_id) = session_id {
                    if let Some(agent) = self.agent_sessions.get_mut(session_id.as_str()) {
                        agent.disconnected = true;
                        agent.detail = Some("bridge disconnected".to_string());
                    }
                    self.set_status(&format!("Agent disconnected: {session_id}"));
                } else {
                    self.set_status("Agent bridge disconnected");
                }
            }
            AgentBridgeEvent::DecodeError(error) => {
                self.set_status(&format!("Agent bridge error: {error}"));
            }
        }
    }

    pub fn terminal_workspace_focused(&self) -> bool {
        matches!(self.focus_target, FocusTarget::TerminalPane(_))
    }

    #[allow(dead_code)]
    pub fn editor_workspace_focused(&self) -> bool {
        !self.terminal_workspace_focused()
    }

    pub fn toggle_workspace_focus(&mut self) {
        if self.terminal_workspace_focused() {
            self.focus_editor();
        } else {
            self.focus_terminal();
        }
    }

    pub fn cycle_terminal_focus(&mut self, forward: bool) -> u64 {
        let pane_id = if forward {
            self.terminal_workspace.focus_next()
        } else {
            self.terminal_workspace.focus_previous()
        };
        self.focus_target = FocusTarget::TerminalPane(pane_id);
        pane_id
    }

    pub fn split_active_terminal(&mut self, axis: SplitAxis) -> anyhow::Result<u64> {
        let pane_id = self.terminal_workspace.split_active(axis);
        let pane = self
            .terminal_workspace
            .pane(pane_id)
            .expect("split pane should exist");
        self.terminal_runtime.ensure_session_with_config(
            pane_id,
            TerminalSessionConfig::new(pane.launcher.clone(), pane.cwd.clone())
                .with_size(self.active_terminal_size())
                .with_scrollback_limit(pane.scrollback_limit),
        )?;
        self.focus_target = FocusTarget::TerminalPane(pane_id);
        Ok(pane_id)
    }

    pub fn close_active_terminal(&mut self) -> bool {
        let Some(pane_id) = self.terminal_workspace.close_active() else {
            return false;
        };
        self.clear_agent_state_for_pane(pane_id);
        self.terminal_runtime.remove_session(pane_id);
        self.focus_target = FocusTarget::TerminalPane(self.terminal_workspace.active_pane_id());
        true
    }

    pub fn resize_terminals_in_workspace(&mut self, area: Rect) {
        for pane_layout in self.terminal_workspace.layout_rects(area) {
            let cols = pane_layout.area.width.saturating_sub(2).max(20);
            let rows = pane_layout.area.height.saturating_sub(2).max(4);
            self.terminal_runtime
                .resize(pane_layout.pane_id, TerminalSize { rows, cols });
        }
    }

    #[allow(dead_code)]
    pub fn send_input_to_active_terminal(&mut self, bytes: &[u8]) -> anyhow::Result<()> {
        self.terminal_runtime
            .send_input(self.terminal_workspace.active_pane_id(), bytes)
    }

    #[allow(dead_code)]
    pub fn scroll_active_terminal(&mut self, delta: isize) {
        self.terminal_runtime
            .scroll(self.terminal_workspace.active_pane_id(), delta);
    }

    pub fn active_terminal_snapshot(&mut self) -> Option<TerminalSnapshot> {
        self.terminal_runtime
            .snapshot(self.terminal_workspace.active_pane_id())
    }

    pub fn active_terminal_workspace_summary(&mut self) -> Option<String> {
        let pane_id = self.terminal_workspace.active_pane_id();
        let pane = self.terminal_workspace.pane(pane_id)?;
        let title = pane.title.clone();
        let cwd_label = pane.cwd_label();
        let snapshot_status = self
            .terminal_runtime
            .snapshot(pane_id)
            .and_then(|snapshot| (!snapshot.status.is_empty()).then_some(snapshot.status));
        let agent_summary = self
            .agent_session_for_pane(pane_id)
            .map(|agent| agent.info_summary());

        let mut parts = vec![format!("{title} · {cwd_label}"), format!("pane {pane_id}")];
        if let Some(agent_summary) = agent_summary {
            parts.push(agent_summary);
        } else if let Some(snapshot_status) = snapshot_status {
            parts.push(snapshot_status);
        }
        Some(parts.join("  "))
    }

    pub fn workspace_summary(&mut self) -> Option<String> {
        if self.terminal_workspace_focused() {
            return self.active_terminal_workspace_summary();
        }

        if self.sidebar_focused {
            let selected = self.file_tree.selected_entry()?;
            let kind = if selected.is_dir { "dir" } else { "file" };
            let label = display_path(&self.root_dir, &selected.path);
            return Some(format!("Files · {kind} · {label}"));
        }

        None
    }

    pub fn active_terminal_application_cursor(&mut self) -> bool {
        self.active_terminal_snapshot()
            .map(|snapshot| snapshot.application_cursor)
            .unwrap_or(false)
    }

    pub fn terminal_pane_renders(&mut self, area: Rect) -> Vec<TerminalPaneRender> {
        let layout_rects = self.terminal_workspace.layout_rects(area);
        let pane_ids: Vec<u64> = layout_rects.iter().map(|layout| layout.pane_id).collect();
        let pane_metadata: HashMap<u64, (String, String, bool)> = pane_ids
            .iter()
            .filter_map(|pane_id| {
                self.terminal_workspace.pane(*pane_id).map(|pane| {
                    (
                        *pane_id,
                        (pane.title.clone(), pane.cwd_label(), pane.id == self.terminal_workspace.active_pane_id()),
                    )
                })
            })
            .collect();

        layout_rects
            .into_iter()
            .filter_map(|pane_layout| {
                let (title, cwd_label, active) = pane_metadata.get(&pane_layout.pane_id)?.clone();
                let agent_status = self
                    .agent_session_for_pane(pane_layout.pane_id)
                    .map(|agent| agent.chrome_status());
                Some(TerminalPaneRender {
                    pane_id: pane_layout.pane_id,
                    area: pane_layout.area,
                    title,
                    cwd_label,
                    agent_status,
                    snapshot: self.terminal_runtime.snapshot(pane_layout.pane_id),
                    active,
                })
            })
            .collect()
    }

    fn active_terminal_size(&self) -> TerminalSize {
        self.terminal_runtime
            .config(self.terminal_workspace.active_pane_id())
            .map(|config| config.size)
            .unwrap_or_default()
    }

    fn can_replace_preview_buffer(&self, idx: usize) -> bool {
        if idx >= self.buffers.len() {
            return false;
        }
        if self.buffers[idx].dirty {
            return false;
        }
        if self
            .split_buffers
            .is_some_and(|(left, right)| left == idx || right == idx)
        {
            return false;
        }
        true
    }

    fn scratch_buffer_index(&self) -> Option<usize> {
        (self.buffers.len() == 1
            && self.preview_buffer.is_none()
            && self.split_buffers.is_none()
            && self.buffers[0].path.is_none()
            && !self.buffers[0].dirty
            && self.buffers[0].content().is_empty())
        .then_some(0)
    }

    fn replace_buffer_with_file(&mut self, idx: usize, path: &Path) -> anyhow::Result<()> {
        let buf = Buffer::from_file(path)?;
        let lang = buf.language.clone();
        self.buffers[idx] = buf;

        if let Some(mut hl) = Highlighter::new(&lang) {
            let content = self.buffers[idx].content();
            hl.parse(&content);
            self.highlighters.insert(idx, hl);
        } else {
            self.highlighters.remove(&idx);
        }

        Ok(())
    }

    fn normalize_workspace_path(&self, path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn agent_session_for_pane(&self, pane_id: u64) -> Option<&AgentSessionState> {
        self.agent_session_bindings
            .iter()
            .find_map(|(session_id, bound_pane_id)| {
                (*bound_pane_id == pane_id)
                    .then(|| self.agent_sessions.get(session_id))
                    .flatten()
            })
    }

    fn agent_session_mut(&mut self, session_id: &str) -> &mut AgentSessionState {
        self.agent_sessions
            .entry(session_id.to_string())
            .or_insert_with(|| AgentSessionState::new(session_id))
    }

    fn clear_agent_state_for_pane(&mut self, pane_id: u64) {
        let bound_sessions: Vec<String> = self
            .agent_session_bindings
            .iter()
            .filter_map(|(session_id, bound_pane_id)| {
                (*bound_pane_id == pane_id).then_some(session_id.clone())
            })
            .collect();
        for session_id in bound_sessions {
            self.agent_session_bindings.remove(&session_id);
            self.agent_sessions.remove(&session_id);
        }
    }

    fn record_touched_file(&mut self, path: &Path) {
        let canon = self.normalize_workspace_path(path);
        self.touched_files.retain(|entry| entry.path != canon);
        let conflict = self.external_conflicts.contains(&canon);
        self.touched_files.insert(
            0,
            TouchedFile {
                path: canon.clone(),
                display_path: display_path(&self.root_dir, &canon),
                conflict,
            },
        );
        self.touched_paths.insert(canon.clone());
        while self.touched_files.len() > 50 {
            if let Some(removed) = self.touched_files.pop() {
                self.touched_paths.remove(&removed.path);
            }
        }
    }

    fn update_touched_conflict(&mut self, path: &Path, conflict: bool) {
        if let Some(entry) = self
            .touched_files
            .iter_mut()
            .find(|entry| entry.path == path)
        {
            entry.conflict = conflict;
        }
    }

    fn reload_buffer_from_disk(&mut self, idx: usize, path: &Path) -> anyhow::Result<bool> {
        let bytes = std::fs::read(path)?;
        let is_binary = bytes.contains(&0);
        let content = String::from_utf8_lossy(&bytes).into_owned();

        if content == self.buffers[idx].content() {
            return Ok(false);
        }

        self.buffers[idx].reload(&content);
        self.buffers[idx].is_binary = is_binary;

        if let Some(hl) = self.highlighters.get_mut(&idx) {
            hl.parse(&content);
        }

        if let Some(path) = self.buffers[idx].path.clone() {
            self.compute_diff_for_path(&path);
        }

        self.refresh_workspace();
        Ok(true)
    }

    fn process_agent_message(&mut self, envelope: Envelope) {
        let Envelope {
            session_id,
            message,
            guidance,
            ..
        } = envelope;
        let session_key = session_id.to_string();
        let _ = self.agent_session_mut(&session_key);

        match message {
            Message::Hello(hello) => {
                let session = self.agent_session_mut(&session_key);
                session.runtime_name = Some(hello.runtime_name.clone());
                session.disconnected = false;
                self.set_status(&format!(
                    "Agent attached: {} ({})",
                    hello.runtime_name, session_id
                ));
            }
            Message::SessionStatus(status) => {
                let status_kind = status.kind.clone();
                let status_label = match status_kind {
                    SessionStatusKind::Starting => "starting",
                    SessionStatusKind::Running => "running",
                    SessionStatusKind::Completed => "completed",
                    SessionStatusKind::Failed => "failed",
                };
                let session = self.agent_session_mut(&session_key);
                session.status_kind = Some(status_kind);
                session.detail = status.detail.clone();
                session.disconnected = false;
                if let Some(detail) = status.detail.filter(|detail| !detail.is_empty()) {
                    self.set_status(&format!("Agent {} {}: {}", session_id, status_label, detail));
                } else {
                    self.set_status(&format!("Agent {} {}", session_id, status_label));
                }
            }
            Message::SubagentLifecycle(lifecycle) => {
                let lifecycle_label = match lifecycle.kind.clone() {
                    SubagentLifecycleKind::Started => "started",
                    SubagentLifecycleKind::Finished => "finished",
                    SubagentLifecycleKind::Failed => "failed",
                };
                let session = self.agent_session_mut(&session_key);
                session.record_subagent(&lifecycle);
                self.set_status(&format!(
                    "Sub-agent {} {} ({})",
                    lifecycle.label, lifecycle_label, session_id
                ));
            }
            Message::EditorAction(action) => match action {
                EditorAction::OpenFile { path } => {
                    if let Some(full_path) = self.resolve_workspace_path(&path) {
                        let _ = self.open_file(&full_path);
                        self.focus_editor();
                        self.set_status(&format!(
                            "Agent opened {}",
                            display_path(&self.root_dir, &full_path)
                        ));
                    }
                }
                EditorAction::RevealPath { path } => {
                    if let Some(full_path) = self.resolve_workspace_path(&path) {
                        self.file_tree.reveal_path(&full_path);
                        self.focus_editor();
                        self.set_status(&format!(
                            "Agent revealed {}",
                            display_path(&self.root_dir, &full_path)
                        ));
                    }
                }
                EditorAction::ShowDiff { path } => {
                    if let Some(full_path) = self.resolve_workspace_path(&path) {
                        let _ = self.open_file(&full_path);
                        self.focus_editor();
                        self.diff_mode = true;
                        self.compute_diff_for_current();
                        self.set_status(&format!(
                            "Agent diffed {}",
                            display_path(&self.root_dir, &full_path)
                        ));
                    }
                }
            },
            Message::HelloAck(_) | Message::WorkspaceQuery(_) | Message::WorkspaceSnapshot(_)
            | Message::ActionResult(_) => {}
        }

        if let Some(guidance) = guidance {
            self.process_guided_editor_action(&session_key, guidance);
        }
    }

    fn process_guided_editor_action(
        &mut self,
        session_id: &str,
        guidance: GuidedEditorAction,
    ) {
        match guidance {
            GuidedEditorAction::FocusEditor => {
                self.focus_editor();
                self.set_status(&format!("Agent focused editor ({session_id})"));
            }
            GuidedEditorAction::SelectText { path, selection } => {
                if let Some(full_path) = self.resolve_workspace_path(&path) {
                    if self.open_file(&full_path).is_ok() {
                        self.focus_editor();
                        self.apply_guided_selection(selection);
                        self.set_status(&format!(
                            "Agent selected {}",
                            display_path(&self.root_dir, &full_path)
                        ));
                    }
                }
            }
        }
    }

    fn apply_guided_selection(&mut self, selection: TextSelection) {
        let start_line = selection.start.line.saturating_sub(1) as usize;
        let start_col = selection
            .start
            .column
            .map(|column| column.saturating_sub(1) as usize)
            .unwrap_or(0);

        if let Some(end) = selection.end {
            let end_line = end.line.saturating_sub(1) as usize;
            let end_col = end
                .column
                .map(|column| column.saturating_sub(1) as usize)
                .unwrap_or(usize::MAX);
            self.current_buffer_mut()
                .select_range(start_line, start_col, end_line, end_col);
        } else {
            let buffer = self.current_buffer_mut();
            buffer.select_range(start_line, start_col, start_line, start_col);
            buffer.clear_selection();
        }

        let height = self.viewport_height.max(1);
        self.current_buffer_mut().ensure_cursor_visible(height);
    }

    fn resolve_workspace_path(&self, path: &str) -> Option<PathBuf> {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            Some(path)
        } else {
            Some(self.root_dir.join(path))
        }
    }

    pub fn breadcrumb(&self) -> String {
        let Some(path) = self.buffers[self.active_buffer].path.as_ref() else {
            return "[untitled]".to_string();
        };
        path.strip_prefix(&self.root_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    pub fn matching_bracket(&self) -> Option<(usize, usize)> {
        let buf = &self.buffers[self.active_buffer];
        let line = buf.get_line(buf.cursor_line)?;
        let chars: Vec<char> = line.chars().collect();

        for &idx in &[buf.cursor_col, buf.cursor_col.saturating_sub(1)] {
            let Some(&ch) = chars.get(idx) else { continue };
            let (open, close, direction) = match ch {
                '(' => ('(', ')', 1isize),
                '[' => ('[', ']', 1isize),
                '{' => ('{', '}', 1isize),
                ')' => ('(', ')', -1isize),
                ']' => ('[', ']', -1isize),
                '}' => ('{', '}', -1isize),
                _ => continue,
            };
            let mut depth = 0isize;
            let mut pos = idx as isize;
            while pos >= 0 && (pos as usize) < chars.len() {
                let current = chars[pos as usize];
                if current == open {
                    depth += if direction > 0 { 1 } else { -1 };
                } else if current == close {
                    depth += if direction > 0 { -1 } else { 1 };
                }
                if depth == 0 && pos as usize != idx {
                    return Some((buf.cursor_line, pos as usize));
                }
                pos += direction;
            }
        }

        None
    }
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut components: Vec<_> = paths.first()?.components().collect();
    for path in &paths[1..] {
        let path_components: Vec<_> = path.components().collect();
        let shared = components
            .iter()
            .zip(path_components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        components.truncate(shared);
    }
    if components.is_empty() {
        return None;
    }
    let mut ancestor = PathBuf::new();
    for component in components {
        ancestor.push(component.as_os_str());
    }
    Some(ancestor)
}

fn remap_split_indices(left: usize, right: usize, removed: usize) -> Option<(usize, usize)> {
    let remap = |idx: usize| -> Option<usize> {
        if idx == removed {
            None
        } else if idx > removed {
            Some(idx - 1)
        } else {
            Some(idx)
        }
    };

    let left = remap(left)?;
    let right = remap(right)?;
    if left == right {
        None
    } else {
        Some((left, right))
    }
}

fn remap_buffer_index(index: Option<usize>, removed: usize) -> Option<usize> {
    index.and_then(|idx| {
        if idx == removed {
            None
        } else if idx > removed {
            Some(idx - 1)
        } else {
            Some(idx)
        }
    })
}

fn ensure_runtime_matches_workspace(
    terminal_workspace: &TerminalWorkspaceState,
    terminal_runtime: &mut TerminalRuntime,
    root_dir: &Path,
) -> anyhow::Result<()> {
    for pane in terminal_workspace.panes().values() {
        terminal_runtime.ensure_session_with_config(
            pane.id,
            TerminalSessionConfig::new(pane.launcher.clone(), pane.cwd.clone())
                .with_scrollback_limit(pane.scrollback_limit),
        )?;
    }
    if terminal_workspace.panes().is_empty() {
        terminal_runtime.ensure_session(1, TerminalLauncher::Shell, root_dir)?;
    }
    Ok(())
}

fn display_path(root_dir: &Path, path: &Path) -> String {
    let normalized_root = root_dir
        .canonicalize()
        .unwrap_or_else(|_| root_dir.to_path_buf());
    path.strip_prefix(&normalized_root)
        .unwrap_or(path)
        .display()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{common_ancestor, remap_split_indices, AppState, ExternalChangeOutcome};
    use crate::agent_bridge::AgentBridgeEvent;
    use crate::workspace::SplitAxis;
    use core_agent_protocol::{
        Envelope, GuidedEditorAction, Message, SessionId, SessionStatus, SessionStatusKind,
        TextSelection,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn common_ancestor_handles_multiple_roots() {
        let roots = vec![
            PathBuf::from("/tmp/work/a"),
            PathBuf::from("/tmp/work/b"),
            PathBuf::from("/tmp/work/c/nested"),
        ];
        assert_eq!(common_ancestor(&roots), Some(PathBuf::from("/tmp/work")));
    }

    #[test]
    fn split_indices_remap_when_tab_closes() {
        assert_eq!(remap_split_indices(1, 3, 0), Some((0, 2)));
        assert_eq!(remap_split_indices(1, 3, 1), None);
        assert_eq!(remap_split_indices(2, 3, 2), None);
    }

    #[test]
    fn workspace_focus_toggles_between_terminal_and_editor() {
        let dir = test_dir("workspace-focus");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        assert!(state.editor_workspace_focused());

        state.focus_terminal();
        assert!(state.terminal_workspace_focused());

        state.toggle_workspace_focus();
        assert!(state.editor_workspace_focused());
        assert!(!state.sidebar_focused);
    }

    #[test]
    fn splitting_terminal_creates_runtime_session_and_close_removes_it() {
        let dir = test_dir("terminal-split");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        let pane_id = state.split_active_terminal(SplitAxis::Vertical).unwrap();
        assert!(state.terminal_runtime.has_session(pane_id));
        assert_eq!(state.terminal_workspace.panes().len(), 2);

        assert!(state.close_active_terminal());
        assert!(!state.terminal_runtime.has_session(pane_id));
        assert_eq!(state.terminal_workspace.panes().len(), 1);
    }

    #[test]
    fn external_changes_mark_conflict_for_dirty_buffers() {
        let dir = test_dir("external-conflict");
        let file_path = dir.join("file.txt");
        fs::write(&file_path, "hello\n").unwrap();

        let mut state = AppState::new(vec![file_path.clone()]).unwrap();
        state.current_buffer_mut().insert_char('!');

        fs::write(&file_path, "changed on disk\n").unwrap();
        let outcome = state.handle_external_change(&file_path);

        assert_eq!(outcome, ExternalChangeOutcome::Conflict("file.txt".to_string()));
        assert!(state.current_buffer_has_external_conflict());
        assert_eq!(state.current_buffer().content(), "!hello\n");
        assert!(state.touched_files[0].conflict);
    }

    #[test]
    fn clean_buffers_reload_and_record_touched_file() {
        let dir = test_dir("external-reload");
        let file_path = dir.join("file.txt");
        fs::write(&file_path, "hello\n").unwrap();

        let mut state = AppState::new(vec![file_path.clone()]).unwrap();
        fs::write(&file_path, "changed on disk\n").unwrap();
        let outcome = state.handle_external_change(&file_path);

        assert_eq!(outcome, ExternalChangeOutcome::Reloaded("file.txt".to_string()));
        assert_eq!(state.current_buffer().content(), "changed on disk\n");
        assert_eq!(state.touched_files[0].display_path, "file.txt");
        assert!(!state.current_buffer_has_external_conflict());
    }

    #[test]
    fn preview_file_reuses_single_buffer_while_browsing() {
        let dir = test_dir("preview-reuse");
        let alpha = dir.join("alpha.txt");
        let beta = dir.join("beta.txt");
        fs::write(&alpha, "alpha\n").unwrap();
        fs::write(&beta, "beta\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        let first = state.preview_file(&alpha).unwrap();
        let second = state.preview_file(&beta).unwrap();

        assert_eq!(first, 0);
        assert_eq!(second, 0);
        assert_eq!(state.buffers.len(), 1);
        assert_eq!(state.preview_buffer, Some(0));
        assert_eq!(state.current_buffer().file_name(), "beta.txt");
    }

    #[test]
    fn preview_file_exits_compare_view() {
        let dir = test_dir("preview-exits-compare");
        let left = dir.join("left.txt");
        let right = dir.join("right.txt");
        let third = dir.join("third.txt");
        fs::write(&left, "left\n").unwrap();
        fs::write(&right, "right\n").unwrap();
        fs::write(&third, "third\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        let left_idx = state.open_file_with_index(&left).unwrap();
        let right_idx = state.open_file_with_index(&right).unwrap();
        state.enter_split(left_idx, right_idx);

        state.preview_file(&third).unwrap();

        assert!(state.split_buffers.is_none());
        assert_eq!(state.current_buffer().file_name(), "third.txt");
    }

    #[test]
    fn guided_selection_opens_file_focuses_editor_and_sets_selection() {
        let dir = test_dir("guided-selection");
        let file_path = dir.join("nested.txt");
        fs::write(&file_path, "alpha\nbeta\ncharlie\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_terminal();
        state.viewport_height = 10;
        state.process_agent_bridge_event(AgentBridgeEvent::Message(
            Envelope::new(
                SessionId::new("pane-1").unwrap(),
                Message::SessionStatus(SessionStatus {
                    kind: SessionStatusKind::Running,
                    detail: Some("selecting".to_string()),
                }),
            )
            .with_guidance(GuidedEditorAction::SelectText {
                path: "nested.txt".to_string(),
                selection: TextSelection::range(2, Some(1), 2, Some(5)),
            }),
        ));

        assert!(state.editor_workspace_focused());
        assert_eq!(state.current_buffer().file_name(), "nested.txt");
        let selection = state
            .current_buffer()
            .selection
            .as_ref()
            .expect("selection should be present");
        assert_eq!(selection.start_line, 1);
        assert_eq!(selection.start_col, 0);
        assert_eq!(selection.end_line, 1);
        assert_eq!(selection.end_col, 4);
    }

    #[test]
    fn guided_focus_switches_back_to_editor() {
        let dir = test_dir("guided-focus");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.focus_terminal();
        state.process_agent_bridge_event(AgentBridgeEvent::Message(
            Envelope::new(
                SessionId::new("pane-1").unwrap(),
                Message::SessionStatus(SessionStatus {
                    kind: SessionStatusKind::Running,
                    detail: Some("focus".to_string()),
                }),
            )
            .with_guidance(GuidedEditorAction::FocusEditor),
        ));

        assert!(state.editor_workspace_focused());
    }

    #[test]
    fn closing_terminal_clears_agent_binding_state() {
        let dir = test_dir("agent-binding-cleanup");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        let pane_id = state.split_active_terminal(SplitAxis::Vertical).unwrap();
        state
            .agent_session_bindings
            .insert("pane-2".to_string(), pane_id);
        state
            .agent_sessions
            .insert("pane-2".to_string(), super::AgentSessionState::new("pane-2"));

        assert!(state.close_active_terminal());
        assert!(!state.agent_session_bindings.contains_key("pane-2"));
        assert!(!state.agent_sessions.contains_key("pane-2"));
    }

    #[test]
    fn terminal_workspace_summary_prefers_agent_status_when_bound() {
        let dir = test_dir("terminal-summary");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.focus_terminal();
        state
            .agent_session_bindings
            .insert("pane-1".to_string(), state.terminal_workspace.active_pane_id());
        let mut session = super::AgentSessionState::new("pane-1");
        session.runtime_name = Some("edit-agent".to_string());
        session.status_kind = Some(SessionStatusKind::Running);
        session.detail = Some("bridge online".to_string());
        state.agent_sessions.insert("pane-1".to_string(), session);

        let summary = state
            .active_terminal_workspace_summary()
            .expect("terminal summary should exist");
        assert!(summary.contains("edit-agent"));
        assert!(summary.contains("bridge online"));

        let panes = state.terminal_pane_renders(ratatui::layout::Rect::new(0, 0, 60, 20));
        assert_eq!(panes.len(), 1);
        assert_eq!(panes[0].agent_status.as_deref(), Some("running"));
    }

    #[test]
    fn workspace_summary_reports_sidebar_selection_when_sidebar_focused() {
        let dir = test_dir("sidebar-summary");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let file_path = dir.join("nested/file.txt");
        fs::write(&file_path, "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.file_tree.reveal_path(&file_path);
        state.focus_sidebar();

        let summary = state.workspace_summary().expect("sidebar summary should exist");
        assert!(summary.contains("Files"));
        assert!(summary.contains("file"));
        assert!(summary.contains("nested/file.txt"));
    }

    #[test]
    fn disconnected_bridge_marks_agent_session_disconnected() {
        let dir = test_dir("agent-disconnect");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state
            .agent_session_bindings
            .insert("pane-1".to_string(), state.terminal_workspace.active_pane_id());
        let mut session = super::AgentSessionState::new("pane-1");
        session.runtime_name = Some("edit-agent".to_string());
        session.status_kind = Some(SessionStatusKind::Running);
        state.agent_sessions.insert("pane-1".to_string(), session);

        state.process_agent_bridge_event(AgentBridgeEvent::Disconnected {
            session_id: Some(SessionId::new("pane-1").unwrap()),
        });

        let session = state
            .agent_sessions
            .get("pane-1")
            .expect("session should still be tracked");
        assert!(session.disconnected);
        assert_eq!(session.detail.as_deref(), Some("bridge disconnected"));
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("edit-state-tests-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
