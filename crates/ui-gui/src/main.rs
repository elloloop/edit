use anyhow::{Context, Result};
use core_buffer::Buffer;
use core_fs::FileTree;
use core_syntax::Highlighter;
use eframe::egui;
use fnug_vt100 as vt100;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const TERMINAL_SCROLLBACK: usize = 10_000;
const TERMINAL_MIN_COLS: u16 = 40;
const TERMINAL_MIN_ROWS: u16 = 10;
const TERMINAL_FONT_SIZE: f32 = 13.0;
const TERMINAL_GUTTER: f32 = 10.0;
const TERMINAL_RATIO_MIN: f32 = 0.22;
const TERMINAL_RATIO_MAX: f32 = 0.78;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalSize {
    rows: u16,
    cols: u16,
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            rows: 24,
            cols: 100,
        }
    }
}

impl TerminalSize {
    fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

enum TerminalEvent {
    Output(Vec<u8>),
}

struct TerminalPane {
    title: String,
    parser: vt100::Parser,
    rx: Option<mpsc::Receiver<TerminalEvent>>,
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    scrollback: usize,
    size: TerminalSize,
    running: bool,
    launch_kind: AgentKind,
    status: String,
}

impl TerminalPane {
    fn new(kind: AgentKind, cwd: &Path) -> Self {
        let mut pane = Self {
            title: kind.label().to_string(),
            parser: vt100::Parser::new(
                TerminalSize::default().rows,
                TerminalSize::default().cols,
                TERMINAL_SCROLLBACK,
            ),
            rx: None,
            writer: None,
            master: None,
            child: None,
            scrollback: 0,
            size: TerminalSize::default(),
            running: false,
            launch_kind: kind,
            status: String::new(),
        };
        pane.spawn(kind, cwd);
        pane
    }

    fn spawn(&mut self, kind: AgentKind, cwd: &Path) {
        self.shutdown();
        self.launch_kind = kind;
        self.title = kind.label().to_string();
        self.scrollback = 0;
        self.parser = vt100::Parser::new(self.size.rows, self.size.cols, TERMINAL_SCROLLBACK);

        match spawn_terminal(kind, cwd, self.size) {
            Ok(session) => {
                self.rx = Some(session.rx);
                self.writer = Some(session.writer);
                self.master = Some(session.master);
                self.child = Some(session.child);
                self.running = true;
                self.status = format!("Running {} in {}", kind.label(), cwd.display());
            }
            Err(err) => {
                self.rx = None;
                self.writer = None;
                self.master = None;
                self.child = None;
                self.running = false;
                self.status = format!("Failed to launch {}: {err:#}", kind.label());
                self.parser.process(self.status.as_bytes());
            }
        }
    }

    fn shutdown(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
        self.child = None;
        self.writer = None;
        self.master = None;
        self.rx = None;
        self.running = false;
    }

    fn poll(&mut self) {
        if let Some(ref rx) = self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    TerminalEvent::Output(bytes) => {
                        self.parser.process(&bytes);
                        if self.scrollback == 0 {
                            self.parser.set_scrollback(0);
                        }
                    }
                }
            }
        }

        if let Some(child) = self.child.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.running = false;
                    self.status = format!("{} exited: {status}", self.title);
                    self.parser
                        .process(format!("\r\n[{}]\r\n", self.status).as_bytes());
                    self.child = None;
                    self.writer = None;
                    self.master = None;
                }
                Ok(None) => {}
                Err(err) => {
                    self.running = false;
                    self.status = format!("{} wait failed: {err}", self.title);
                    self.parser
                        .process(format!("\r\n[{}]\r\n", self.status).as_bytes());
                    self.child = None;
                    self.writer = None;
                    self.master = None;
                }
            }
        }
    }

    fn resize(&mut self, size: TerminalSize) {
        if size == self.size {
            return;
        }
        self.size = size;
        self.parser.set_size(size.rows, size.cols);
        self.parser.set_scrollback(self.scrollback);
        if let Some(master) = self.master.as_ref() {
            let _ = master.resize(size.pty_size());
        }
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    fn handle_events(&mut self, events: &[egui::Event]) -> bool {
        let mut sent = false;
        for event in events {
            if let Some(bytes) =
                translate_terminal_event(event, self.parser.screen().application_cursor())
            {
                self.send_bytes(&bytes);
                sent = true;
            }
        }
        sent
    }

    fn scroll(&mut self, delta_rows: isize) {
        let len = self.parser.scrollback_len() as isize;
        let next = (self.scrollback as isize + delta_rows).clamp(0, len);
        self.scrollback = next as usize;
        self.parser.set_scrollback(self.scrollback);
    }

    fn scroll_to_bottom(&mut self) {
        self.scrollback = 0;
        self.parser.set_scrollback(0);
    }

    fn screen_text(&mut self) -> String {
        self.parser.set_scrollback(self.scrollback);
        let mut text = self.parser.screen().contents();
        if text.is_empty() {
            text.push('\n');
        }
        text
    }
}

impl Drop for TerminalPane {
    fn drop(&mut self) {
        self.shutdown();
    }
}

struct SpawnedTerminal {
    rx: mpsc::Receiver<TerminalEvent>,
    writer: Box<dyn Write + Send>,
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

fn spawn_terminal(kind: AgentKind, cwd: &Path, size: TerminalSize) -> Result<SpawnedTerminal> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(size.pty_size())
        .context("create pty pair")?;

    let (program, args) = kind.command();
    let mut command = CommandBuilder::new(&program);
    command.args(args.iter().copied());
    command.cwd(cwd);
    command.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(command)
        .with_context(|| format!("spawn {program}"))?;
    let writer = pair.master.take_writer().context("open terminal writer")?;
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("open terminal reader")?;
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(TerminalEvent::Output(buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    Ok(SpawnedTerminal {
        rx,
        writer,
        master: pair.master,
        child,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AgentKind {
    Claude,
    OpenCode,
    Goose,
    Shell,
}

impl AgentKind {
    fn all() -> [Self; 4] {
        [Self::Shell, Self::Claude, Self::Goose, Self::OpenCode]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::OpenCode => "OpenCode",
            Self::Goose => "Goose",
            Self::Shell => "Shell",
        }
    }

    fn command(self) -> (String, Vec<&'static str>) {
        match self {
            Self::Claude => ("claude".to_string(), vec![]),
            Self::OpenCode => ("opencode".to_string(), vec![]),
            Self::Goose => ("goose".to_string(), vec![]),
            Self::Shell => (
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
                vec![],
            ),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalSplit {
    Vertical,
    Horizontal,
}

impl TerminalSplit {
    fn label(self) -> &'static str {
        match self {
            Self::Vertical => "Vertical",
            Self::Horizontal => "Horizontal",
        }
    }
}

enum EditorAction {
    OpenFile(PathBuf),
    ToggleDirectory(PathBuf),
    SelectBuffer(usize),
}

struct EditApp {
    buffers: Vec<Buffer>,
    active_buffer: usize,
    compare_buffer: Option<usize>,
    file_tree: Option<FileTree>,
    highlighters: HashMap<usize, Highlighter>,
    sidebar_visible: bool,
    root_dir: PathBuf,
    command_input: String,
    status_message: Option<String>,
    wrap_lines: bool,
    editing: bool,
    terminal_split: TerminalSplit,
    terminal_panes: Vec<TerminalPane>,
    active_terminal: usize,
    terminal_ratio: f32,

    file_rx: Option<mpsc::Receiver<core_fs::FileEvent>>,
    #[allow(dead_code)]
    file_watcher: Option<core_fs::FileWatcherHandle>,
}

impl EditApp {
    fn new(paths: Vec<PathBuf>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        let normalized_paths: Vec<PathBuf> = if paths.is_empty() {
            vec![cwd.clone()]
        } else {
            paths
                .into_iter()
                .map(|path| {
                    if path.is_absolute() {
                        path
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
        let mut file_tree = FileTree::build_multi(&roots, &root_dir).ok();

        let mut buffers = Vec::new();
        let mut highlighters = HashMap::new();
        for file_path in initial_files {
            if let Ok(buf) = Buffer::from_file(&file_path) {
                let lang = buf.language.clone();
                buffers.push(buf);
                let idx = buffers.len() - 1;
                if let Some(mut hl) = Highlighter::new(&lang) {
                    let content = buffers[idx].content();
                    hl.parse(&content);
                    highlighters.insert(idx, hl);
                }
            }
        }

        if buffers.is_empty() {
            buffers.push(Buffer::from_string(""));
        }

        if let Some(path) = buffers.first().and_then(|buffer| buffer.path.as_deref()) {
            if let Some(tree) = file_tree.as_mut() {
                tree.reveal_path(path);
            }
        }

        let (tx, rx) = mpsc::channel();
        let watcher = core_fs::watch_directory(&root_dir, tx).ok();

        Self {
            buffers,
            active_buffer: 0,
            compare_buffer: None,
            file_tree,
            highlighters,
            sidebar_visible: true,
            root_dir: root_dir.clone(),
            command_input: String::new(),
            status_message: None,
            wrap_lines: false,
            editing: false,
            terminal_split: TerminalSplit::Vertical,
            terminal_panes: vec![TerminalPane::new(AgentKind::Shell, &root_dir)],
            active_terminal: 0,
            terminal_ratio: 0.45,
            file_rx: Some(rx),
            file_watcher: watcher,
        }
    }

    fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.active_buffer]
    }

    fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active_buffer]
    }

    fn active_terminal_mut(&mut self) -> Option<&mut TerminalPane> {
        self.terminal_panes.get_mut(self.active_terminal)
    }

    fn open_file(&mut self, path: &Path) {
        let _ = self.open_file_with_index(path);
    }

    fn open_file_with_index(&mut self, path: &Path) -> Option<usize> {
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path) {
                self.active_buffer = i;
                if let Some(tree) = self.file_tree.as_mut() {
                    tree.reveal_path(path);
                }
                return Some(i);
            }
        }

        if let Ok(buf) = Buffer::from_file(path) {
            let lang = buf.language.clone();
            self.buffers.push(buf);
            let idx = self.buffers.len() - 1;
            self.active_buffer = idx;
            if let Some(mut hl) = Highlighter::new(&lang) {
                let content = self.buffers[idx].content();
                hl.parse(&content);
                self.highlighters.insert(idx, hl);
            }
            if let Some(tree) = self.file_tree.as_mut() {
                tree.reveal_path(path);
            }
            Some(idx)
        } else {
            self.status_message = Some(format!("Failed to open {}", path.display()));
            None
        }
    }

    fn process_file_events(&mut self) {
        let mut changed = vec![];
        let mut refresh_tree = false;
        if let Some(ref rx) = self.file_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    core_fs::FileEvent::Modified(path) | core_fs::FileEvent::Created(path) => {
                        changed.push(path);
                        refresh_tree = true;
                    }
                    core_fs::FileEvent::Deleted(_) => refresh_tree = true,
                }
            }
        }

        for pane in &mut self.terminal_panes {
            pane.poll();
        }

        for path in changed {
            let canon = path.canonicalize().unwrap_or(path);
            let idx = self.buffers.iter().position(|buf| {
                buf.path
                    .as_ref()
                    .is_some_and(|p| p.canonicalize().unwrap_or(p.clone()) == canon)
            });
            if let Some(idx) = idx {
                if !self.buffers[idx].dirty {
                    if let Ok(bytes) = std::fs::read(&canon) {
                        let content = String::from_utf8_lossy(&bytes).into_owned();
                        if content != self.buffers[idx].content() {
                            let name = self.buffers[idx].file_name();
                            self.buffers[idx].reload(&content);
                            self.buffers[idx].is_binary = bytes.contains(&0);
                            if let Some(hl) = self.highlighters.get_mut(&idx) {
                                hl.parse(&content);
                            }
                            self.status_message = Some(format!("Reloaded: {name}"));
                        }
                    }
                }
            }
        }

        if refresh_tree {
            if let Some(tree) = self.file_tree.as_mut() {
                tree.refresh();
            }
        }
    }

    fn relaunch_active_terminal(&mut self, kind: AgentKind) {
        let root = self.root_dir.clone();
        if let Some(pane) = self.active_terminal_mut() {
            pane.spawn(kind, &root);
            self.status_message = Some(format!("Launched {} in active terminal", kind.label()));
        }
    }

    fn split_terminal(&mut self, split: TerminalSplit) {
        self.terminal_split = split;
        self.terminal_panes.insert(
            self.active_terminal + 1,
            TerminalPane::new(AgentKind::Shell, &self.root_dir),
        );
        self.active_terminal += 1;
        self.status_message = Some(format!("Opened {} split", split.label().to_lowercase()));
    }

    fn close_active_terminal(&mut self) {
        if self.terminal_panes.len() <= 1 {
            self.status_message = Some("At least one terminal pane must remain".to_string());
            return;
        }
        self.terminal_panes.remove(self.active_terminal);
        if self.active_terminal >= self.terminal_panes.len() {
            self.active_terminal = self.terminal_panes.len() - 1;
        }
        self.status_message = Some("Closed terminal split".to_string());
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
                match self.current_buffer_mut().save() {
                    Ok(()) => self.status_message = Some("Saved".to_string()),
                    Err(e) => self.status_message = Some(format!("Save failed: {e}")),
                }
            }
            if input.consume_key(egui::Modifiers::COMMAND, egui::Key::Z) {
                if self.current_buffer_mut().undo() {
                    self.status_message = Some("Undo".to_string());
                }
            }
            if input.consume_key(
                egui::Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                },
                egui::Key::Z,
            ) {
                if self.current_buffer_mut().redo() {
                    self.status_message = Some("Redo".to_string());
                }
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                if self.compare_buffer.is_some() {
                    self.compare_buffer = None;
                    self.status_message = Some("Exited compare view".to_string());
                } else if self.editing {
                    self.editing = false;
                    self.status_message = Some("View mode".to_string());
                } else if let Some(pane) = self.active_terminal_mut() {
                    pane.scroll_to_bottom();
                }
            }
        });
    }

    fn execute_command(&mut self) {
        let input = self.command_input.trim().to_string();
        self.command_input.clear();
        if input.is_empty() {
            return;
        }

        match input.as_str() {
            "edit" => {
                self.editing = true;
                self.status_message = Some("Edit mode".to_string());
            }
            "wrap" => {
                self.wrap_lines = !self.wrap_lines;
                self.status_message = Some(if self.wrap_lines {
                    "Word wrap on".to_string()
                } else {
                    "Word wrap off".to_string()
                });
            }
            "close" => {
                if self.compare_buffer.is_some() {
                    self.compare_buffer = None;
                    self.status_message = Some("Exited compare view".to_string());
                }
            }
            other if other.starts_with("compare ") => {
                let parts: Vec<&str> = other["compare ".len()..].split_whitespace().collect();
                if parts.len() == 2 {
                    let left = self.open_file_with_index(&self.root_dir.join(parts[0]));
                    let right = self.open_file_with_index(&self.root_dir.join(parts[1]));
                    if let (Some(left), Some(right)) = (left, right) {
                        self.active_buffer = left;
                        self.compare_buffer = Some(right);
                        self.status_message = Some("Compare view".to_string());
                    }
                } else {
                    self.status_message = Some("Usage: compare <file1> <file2>".to_string());
                }
            }
            other if other.starts_with("open ") => {
                self.open_file(&self.root_dir.join(other["open ".len()..].trim()));
            }
            _ => {
                self.status_message = Some(format!("Unknown command: {input}"));
            }
        }
    }

    fn breadcrumb(&self) -> String {
        self.current_buffer()
            .path
            .as_ref()
            .map(|path| {
                path.strip_prefix(&self.root_dir)
                    .unwrap_or(path)
                    .display()
                    .to_string()
            })
            .unwrap_or_else(|| "[untitled]".to_string())
    }
}

impl eframe::App for EditApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_file_events();
        self.handle_shortcuts(ctx);
        ctx.request_repaint_after(Duration::from_millis(16));

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Terminal", |ui| {
                    for kind in AgentKind::all() {
                        if ui.button(format!("Launch {}", kind.label())).clicked() {
                            self.relaunch_active_terminal(kind);
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Split Vertical").clicked() {
                        self.split_terminal(TerminalSplit::Vertical);
                        ui.close_menu();
                    }
                    if ui.button("Split Horizontal").clicked() {
                        self.split_terminal(TerminalSplit::Horizontal);
                        ui.close_menu();
                    }
                    if ui.button("Close Active Split").clicked() {
                        self.close_active_terminal();
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .button(if self.sidebar_visible {
                            "Hide File Tree"
                        } else {
                            "Show File Tree"
                        })
                        .clicked()
                    {
                        self.sidebar_visible = !self.sidebar_visible;
                        ui.close_menu();
                    }
                    if ui
                        .button(if self.wrap_lines {
                            "Disable Wrap"
                        } else {
                            "Enable Wrap"
                        })
                        .clicked()
                    {
                        self.wrap_lines = !self.wrap_lines;
                        ui.close_menu();
                    }
                });
                if ui.button("Quit").clicked() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(13, 147, 115), ">");
                let response = ui.text_edit_singleline(&mut self.command_input);
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.command_input.is_empty()
                {
                    self.execute_command();
                }
                ui.separator();
                let buf = self.current_buffer();
                ui.label(format!(
                    "{}  {}  Ln {}, Col {}{}{}",
                    buf.file_name(),
                    buf.language,
                    buf.cursor_line + 1,
                    buf.cursor_col + 1,
                    if self.compare_buffer.is_some() {
                        "  compare"
                    } else {
                        ""
                    },
                    if self.editing { "  EDIT" } else { "" }
                ));
                if let Some(pane) = self.terminal_panes.get(self.active_terminal) {
                    ui.separator();
                    ui.label(format!(
                        "{}  {}x{}{}",
                        pane.title,
                        pane.size.cols,
                        pane.size.rows,
                        if pane.running { "" } else { "  stopped" }
                    ));
                }
                if let Some(ref msg) = self.status_message {
                    ui.separator();
                    ui.label(msg);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_width = ui.available_width();
            let height = ui.available_height();
            let divider_width = 8.0;
            let left_width = (total_width * self.terminal_ratio).max(320.0);
            let right_width = (total_width - left_width - divider_width).max(420.0);
            let breadcrumb = self.breadcrumb();
            let terminal_input_events = ctx.input(|input| input.events.clone());

            ui.horizontal(|ui| {
                ui.allocate_ui(egui::vec2(left_width, height), |ui| {
                    render_terminal_workspace(
                        ui,
                        &mut self.terminal_panes,
                        &mut self.active_terminal,
                        self.terminal_split,
                        &terminal_input_events,
                        &self.root_dir,
                    );
                });

                let (divider_rect, divider_response) =
                    ui.allocate_exact_size(egui::vec2(divider_width, height), egui::Sense::drag());
                ui.painter().rect_filled(
                    divider_rect.shrink2(egui::vec2(2.5, 0.0)),
                    2.0,
                    if divider_response.dragged() {
                        egui::Color32::from_rgb(75, 124, 233)
                    } else {
                        ui.visuals().widgets.noninteractive.bg_stroke.color
                    },
                );
                if divider_response.dragged() {
                    let delta = ctx.input(|input| input.pointer.delta().x);
                    self.terminal_ratio = (self.terminal_ratio + delta / total_width)
                        .clamp(TERMINAL_RATIO_MIN, TERMINAL_RATIO_MAX);
                }

                ui.allocate_ui(egui::vec2(right_width, height), |ui| {
                    if let Some(action) = render_editor_workspace(
                        ui,
                        &mut self.buffers,
                        self.active_buffer,
                        self.compare_buffer,
                        self.sidebar_visible,
                        self.file_tree.as_ref(),
                        self.wrap_lines,
                        self.editing,
                        &breadcrumb,
                    ) {
                        match action {
                            EditorAction::OpenFile(path) => self.open_file(&path),
                            EditorAction::ToggleDirectory(path) => {
                                if let Some(tree) = self.file_tree.as_mut() {
                                    if let Some(idx) =
                                        tree.entries.iter().position(|entry| entry.path == path)
                                    {
                                        tree.toggle_expand(idx);
                                    }
                                }
                            }
                            EditorAction::SelectBuffer(index) => {
                                if index < self.buffers.len() {
                                    self.active_buffer = index;
                                }
                            }
                        }
                    }
                });
            });
        });
    }
}

fn render_terminal_workspace(
    ui: &mut egui::Ui,
    panes: &mut [TerminalPane],
    active_terminal: &mut usize,
    split: TerminalSplit,
    input_events: &[egui::Event],
    root_dir: &Path,
) {
    ui.horizontal(|ui| {
        ui.strong("Terminal Workspace");
        ui.separator();
        ui.label(format!("{} split", split.label().to_lowercase()));
        ui.separator();
        ui.label(root_dir.display().to_string());
    });
    ui.separator();

    match split {
        TerminalSplit::Vertical => {
            ui.columns(panes.len().max(1), |columns| {
                for (idx, pane) in panes.iter_mut().enumerate() {
                    if let Some(column) = columns.get_mut(idx) {
                        render_terminal_pane(column, pane, idx, active_terminal, input_events);
                    }
                }
            });
        }
        TerminalSplit::Horizontal => {
            let pane_count = panes.len();
            for (idx, pane) in panes.iter_mut().enumerate() {
                render_terminal_pane(ui, pane, idx, active_terminal, input_events);
                if idx + 1 < pane_count {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.add_space(6.0);
                }
            }
        }
    }
}

fn render_terminal_pane(
    ui: &mut egui::Ui,
    pane: &mut TerminalPane,
    idx: usize,
    active_terminal: &mut usize,
    input_events: &[egui::Event],
) {
    let pane_id = ui.make_persistent_id(("terminal-pane", idx));
    let selected = *active_terminal == idx;
    let frame = egui::Frame::group(ui.style()).fill(if selected {
        egui::Color32::from_rgb(16, 20, 27)
    } else {
        egui::Color32::from_rgb(12, 15, 21)
    });

    let response = frame
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .selectable_label(selected, egui::RichText::new(&pane.title).strong())
                    .clicked()
                {
                    *active_terminal = idx;
                    ui.memory_mut(|memory| memory.request_focus(pane_id));
                }
                ui.separator();
                ui.small(if pane.running { "live" } else { "stopped" });
                if pane.scrollback > 0 {
                    ui.separator();
                    ui.small(format!("scrollback {}", pane.scrollback));
                }
            });
            ui.small(&pane.status);
            ui.separator();

            let available = ui.available_size_before_wrap();
            let screen_height = (available.y - 4.0).max(180.0);
            let terminal_size = estimate_terminal_size(ui, available.x, screen_height);
            pane.resize(terminal_size);

            let (rect, _response) = ui
                .allocate_exact_size(egui::vec2(available.x, screen_height), egui::Sense::click());
            let response = ui.interact(rect, pane_id, egui::Sense::click());
            if response.clicked() {
                *active_terminal = idx;
                ui.memory_mut(|memory| memory.request_focus(pane_id));
            }

            if response.hovered() {
                let scroll = ui.input(|input| input.raw_scroll_delta.y);
                if scroll.abs() > 0.0 {
                    let delta_rows = if scroll > 0.0 { 3 } else { -3 };
                    pane.scroll(delta_rows);
                }
            }

            if response.has_focus() && pane.handle_events(input_events) {
                pane.scroll_to_bottom();
            }

            let text = pane.screen_text();
            let inner_rect = rect.shrink(TERMINAL_GUTTER);
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner_rect), |ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(TERMINAL_FONT_SIZE));
                ui.style_mut().visuals.override_text_color =
                    Some(egui::Color32::from_rgb(220, 226, 236));
                let mut text = text;
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(usize::from(terminal_size.rows))
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });

            response
        })
        .inner;

    if selected && response.has_focus() {
        let stroke = egui::Stroke::new(1.5, egui::Color32::from_rgb(75, 124, 233));
        ui.painter()
            .rect_stroke(response.rect, 6.0, stroke, egui::StrokeKind::Outside);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_editor_workspace(
    ui: &mut egui::Ui,
    buffers: &mut [Buffer],
    active_buffer: usize,
    compare_buffer: Option<usize>,
    sidebar_visible: bool,
    file_tree: Option<&FileTree>,
    wrap_lines: bool,
    editing: bool,
    breadcrumb: &str,
) -> Option<EditorAction> {
    let mut action = None;
    ui.horizontal(|ui| {
        let editor_total_width = ui.available_width();
        let sidebar_width = if sidebar_visible { 240.0 } else { 0.0 };
        if let Some(tree) = file_tree {
            if sidebar_visible {
                let entries: Vec<(PathBuf, String, bool, usize, Option<char>, bool)> = tree
                    .visible_entries()
                    .iter()
                    .map(|entry| {
                        (
                            entry.path.clone(),
                            entry.name.clone(),
                            entry.is_dir,
                            entry.depth,
                            entry.git_status,
                            tree.expanded.contains(&entry.path),
                        )
                    })
                    .collect();
                ui.allocate_ui(egui::vec2(sidebar_width, ui.available_height()), |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Files");
                        ui.separator();
                        ui.small("Explorer");
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (path, name, is_dir, depth, git_status, expanded) in &entries {
                            let indent = "  ".repeat(*depth);
                            let icon = if *is_dir {
                                if *expanded {
                                    "▾"
                                } else {
                                    "▸"
                                }
                            } else {
                                file_icon(name)
                            };
                            let git =
                                git_status.map_or(" ".to_string(), |status| status.to_string());
                            let label = format!("{indent}{icon} {git} {name}");
                            let rich = egui::RichText::new(label)
                                .color(git_color(*git_status))
                                .monospace();
                            if ui.selectable_label(false, rich).clicked() {
                                action = Some(if *is_dir {
                                    EditorAction::ToggleDirectory(path.clone())
                                } else {
                                    EditorAction::OpenFile(path.clone())
                                });
                            }
                        }
                    });
                });
                ui.separator();
            }
        }

        let editor_width = editor_total_width - sidebar_width;
        ui.allocate_ui(
            egui::vec2(editor_width.max(320.0), ui.available_height()),
            |ui| {
                let conflicts = conflicting_names(buffers);
                ui.horizontal_wrapped(|ui| {
                    for (idx, buffer) in buffers.iter().enumerate() {
                        let title = display_tab_name(buffer, &conflicts);
                        let dirty = if buffer.dirty { " *" } else { "" };
                        if ui
                            .selectable_label(active_buffer == idx, format!("{title}{dirty}"))
                            .clicked()
                        {
                            action = Some(EditorAction::SelectBuffer(idx));
                        }
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(breadcrumb).small().monospace());
                    ui.separator();
                    ui.small(if editing { "edit mode" } else { "view mode" });
                    if wrap_lines {
                        ui.separator();
                        ui.small("wrap");
                    }
                });
                ui.separator();

                if let Some(compare_idx) = compare_buffer {
                    ui.columns(2, |columns| {
                        if active_buffer < buffers.len() && compare_idx < buffers.len() {
                            if active_buffer < compare_idx {
                                let (left, right) = buffers.split_at_mut(compare_idx);
                                render_editor_pane(
                                    &mut columns[0],
                                    &mut left[active_buffer],
                                    editing,
                                    wrap_lines,
                                    true,
                                );
                                render_editor_pane(
                                    &mut columns[1],
                                    &mut right[0],
                                    false,
                                    wrap_lines,
                                    false,
                                );
                            } else if compare_idx < active_buffer {
                                let (left, right) = buffers.split_at_mut(active_buffer);
                                render_editor_pane(
                                    &mut columns[0],
                                    &mut right[0],
                                    editing,
                                    wrap_lines,
                                    true,
                                );
                                render_editor_pane(
                                    &mut columns[1],
                                    &mut left[compare_idx],
                                    false,
                                    wrap_lines,
                                    false,
                                );
                            } else {
                                render_editor_pane(
                                    &mut columns[0],
                                    &mut buffers[active_buffer],
                                    editing,
                                    wrap_lines,
                                    true,
                                );
                            }
                        }
                    });
                } else {
                    render_editor_pane(ui, &mut buffers[active_buffer], editing, wrap_lines, true);
                }
            },
        );
    });
    action
}

fn render_editor_pane(
    ui: &mut egui::Ui,
    buffer: &mut Buffer,
    editing: bool,
    wrap_lines: bool,
    active: bool,
) {
    if active {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            render_buffer_contents(ui, buffer, editing, wrap_lines);
        });
    } else {
        render_buffer_contents(ui, buffer, editing, wrap_lines);
    }
}

fn render_buffer_contents(ui: &mut egui::Ui, buffer: &mut Buffer, editing: bool, wrap_lines: bool) {
    if buffer.is_binary {
        ui.label("Binary file preview disabled");
        return;
    }

    let mut text = buffer.content();
    let width = if wrap_lines {
        ui.available_width()
    } else {
        f32::INFINITY
    };

    egui::ScrollArea::both().show(ui, |ui| {
        ui.style_mut().override_font_id = Some(egui::FontId::monospace(13.0));
        let response = ui.add(
            egui::TextEdit::multiline(&mut text)
                .font(egui::TextStyle::Monospace)
                .desired_width(width)
                .desired_rows(30)
                .interactive(editing),
        );
        if editing && response.changed() && text != buffer.content() {
            buffer.replace_content(&text);
        }
    });
}

fn estimate_terminal_size(ui: &egui::Ui, width: f32, height: f32) -> TerminalSize {
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace).max(14.0);
    let col_width =
        ui.fonts(|fonts| fonts.glyph_width(&egui::FontId::monospace(TERMINAL_FONT_SIZE), 'W'));
    let usable_width = (width - (TERMINAL_GUTTER * 2.0)).max(1.0);
    let usable_height = (height - (TERMINAL_GUTTER * 2.0)).max(1.0);
    TerminalSize {
        cols: ((usable_width / col_width).floor() as u16).max(TERMINAL_MIN_COLS),
        rows: ((usable_height / row_height).floor() as u16).max(TERMINAL_MIN_ROWS),
    }
}

fn translate_terminal_event(event: &egui::Event, application_cursor: bool) -> Option<Vec<u8>> {
    match event {
        egui::Event::Text(text) if !text.is_empty() => Some(text.as_bytes().to_vec()),
        egui::Event::Paste(text) if !text.is_empty() => Some(text.as_bytes().to_vec()),
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => translate_key(*key, *modifiers, application_cursor),
        _ => None,
    }
}

fn translate_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
    application_cursor: bool,
) -> Option<Vec<u8>> {
    if modifiers.command {
        return None;
    }
    if modifiers.ctrl {
        let control = match key {
            egui::Key::C => Some(vec![3]),
            egui::Key::D => Some(vec![4]),
            egui::Key::L => Some(vec![12]),
            egui::Key::Z => Some(vec![26]),
            _ => None,
        };
        if control.is_some() {
            return control;
        }
    }

    let arrow = |normal: &'static [u8], app: &'static [u8]| -> Vec<u8> {
        if application_cursor {
            app.to_vec()
        } else {
            normal.to_vec()
        }
    };

    match key {
        egui::Key::Enter => Some(vec![b'\r']),
        egui::Key::Tab => Some(vec![b'\t']),
        egui::Key::Backspace => Some(vec![0x7f]),
        egui::Key::Escape => Some(vec![0x1b]),
        egui::Key::ArrowUp => Some(arrow(b"\x1b[A", b"\x1bOA")),
        egui::Key::ArrowDown => Some(arrow(b"\x1b[B", b"\x1bOB")),
        egui::Key::ArrowRight => Some(arrow(b"\x1b[C", b"\x1bOC")),
        egui::Key::ArrowLeft => Some(arrow(b"\x1b[D", b"\x1bOD")),
        egui::Key::Home => Some(vec![0x1b, b'[', b'H']),
        egui::Key::End => Some(vec![0x1b, b'[', b'F']),
        egui::Key::Insert => Some(vec![0x1b, b'[', b'2', b'~']),
        egui::Key::Delete => Some(vec![0x1b, b'[', b'3', b'~']),
        egui::Key::PageUp => Some(vec![0x1b, b'[', b'5', b'~']),
        egui::Key::PageDown => Some(vec![0x1b, b'[', b'6', b'~']),
        _ => None,
    }
}

fn git_color(status: Option<char>) -> egui::Color32 {
    match status {
        Some('M') => egui::Color32::from_rgb(110, 170, 255),
        Some('A') | Some('?') => egui::Color32::from_rgb(115, 201, 145),
        Some('D') => egui::Color32::from_rgb(228, 103, 107),
        _ => egui::Color32::from_gray(210),
    }
}

fn file_icon(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("rs") => "r",
        Some("md") => "•",
        Some("toml") | Some("json") | Some("yaml") | Some("yml") => "{}",
        Some("js") | Some("ts") | Some("tsx") | Some("jsx") => "◉",
        Some("html") | Some("css") => "◌",
        Some("sh") | Some("bash") | Some("zsh") => "›",
        _ => "·",
    }
}

fn conflicting_names(buffers: &[Buffer]) -> HashSet<String> {
    let mut counts = HashMap::new();
    for buffer in buffers {
        *counts.entry(buffer.file_name()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect()
}

fn display_tab_name(buffer: &Buffer, conflicts: &HashSet<String>) -> String {
    let file_name = buffer.file_name();
    if !conflicts.contains(&file_name) {
        return file_name;
    }
    buffer
        .path
        .as_ref()
        .and_then(|path| path.parent())
        .and_then(|parent| parent.file_name())
        .map(|name| format!("{}/{}", name.to_string_lossy(), file_name))
        .unwrap_or(file_name)
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

fn main() -> eframe::Result {
    let paths: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1680.0, 980.0])
            .with_title("edit"),
        ..Default::default()
    };

    eframe::run_native(
        "edit",
        options,
        Box::new(|_cc| Ok(Box::new(EditApp::new(paths)))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_has_minimums() {
        let size = TerminalSize::default();
        assert!(size.cols >= TERMINAL_MIN_COLS);
        assert!(size.rows >= TERMINAL_MIN_ROWS);
    }

    #[test]
    fn conflicting_tab_names_use_parent_folder() {
        let mut left = Buffer::from_string("left");
        left.path = Some(PathBuf::from("/tmp/a/main.rs"));
        let mut right = Buffer::from_string("right");
        right.path = Some(PathBuf::from("/tmp/b/main.rs"));
        let buffers = [left, right];
        let conflicts = conflicting_names(&buffers);
        assert_eq!(display_tab_name(&buffers[0], &conflicts), "a/main.rs");
        assert_eq!(display_tab_name(&buffers[1], &conflicts), "b/main.rs");
    }

    #[test]
    fn terminal_key_translation_uses_application_cursor_mode() {
        assert_eq!(
            translate_key(egui::Key::ArrowUp, egui::Modifiers::NONE, false),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            translate_key(egui::Key::ArrowUp, egui::Modifiers::NONE, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            translate_key(
                egui::Key::C,
                egui::Modifiers {
                    ctrl: true,
                    ..Default::default()
                },
                false
            ),
            Some(vec![3])
        );
    }

    #[test]
    fn common_ancestor_finds_shared_prefix() {
        let paths = vec![
            PathBuf::from("/tmp/work/a"),
            PathBuf::from("/tmp/work/b"),
            PathBuf::from("/tmp/work/c/file.rs"),
        ];
        assert_eq!(common_ancestor(&paths), Some(PathBuf::from("/tmp/work")));
    }
}
