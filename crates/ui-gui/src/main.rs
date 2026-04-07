use core_buffer::Buffer;
use core_fs::FileTree;
use core_syntax::Highlighter;
use eframe::egui;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;

struct TerminalPane {
    title: String,
    output: String,
    input: String,
    process: Option<Child>,
    stdin: Option<ChildStdin>,
    rx: Option<mpsc::Receiver<String>>,
}

impl TerminalPane {
    fn shell(cwd: &Path) -> Self {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
        let label = Path::new(&shell)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or(shell.clone());
        let mut pane = Self::empty(&label);
        pane.spawn(&shell, &[], cwd);
        pane
    }

    fn empty(label: &str) -> Self {
        Self {
            title: label.to_string(),
            output: String::new(),
            input: String::new(),
            process: None,
            stdin: None,
            rx: None,
        }
    }

    fn spawn(&mut self, cmd: &str, args: &[&str], cwd: &Path) {
        self.output.clear();
        self.input.clear();

        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        let mut child = match Command::new(cmd)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) => {
                let _ = tx.send(format!("Failed to start {cmd}: {err}\n"));
                return;
            }
        };

        self.stdin = child.stdin.take();
        self.process = Some(child);

        if let Some(stdout) = self.process.as_mut().and_then(|child| child.stdout.take()) {
            let tx = tx.clone();
            std::thread::spawn(move || read_stream(stdout, tx));
        }
        if let Some(stderr) = self.process.as_mut().and_then(|child| child.stderr.take()) {
            let tx = tx.clone();
            std::thread::spawn(move || read_stream(stderr, tx));
        }
    }

    fn poll(&mut self) {
        if let Some(ref rx) = self.rx {
            while let Ok(text) = rx.try_recv() {
                self.output.push_str(&text);
                if self.output.len() > 80_000 {
                    let trim = self.output.len() - 60_000;
                    self.output = self.output[trim..].to_string();
                }
            }
        }
    }

    fn send_input(&mut self) {
        let line = self.input.trim_end().to_string();
        if line.is_empty() {
            return;
        }
        if let Some(stdin) = self.stdin.as_mut() {
            let _ = stdin.write_all(line.as_bytes());
            let _ = stdin.write_all(b"\n");
            let _ = stdin.flush();
            self.output.push_str(&format!("$ {line}\n"));
        }
        self.input.clear();
    }
}

fn read_stream<R: Read + Send + 'static>(mut stream: R, tx: mpsc::Sender<String>) {
    let mut buf = [0u8; 4096];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let text = String::from_utf8_lossy(&buf[..n]).to_string();
                if tx.send(text).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[derive(Clone, Copy)]
enum AgentKind {
    Claude,
    OpenCode,
    Goose,
    Shell,
}

impl AgentKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::OpenCode => "opencode",
            Self::Goose => "Goose",
            Self::Shell => "Shell",
        }
    }

    fn command(&self) -> (&'static str, Vec<&'static str>) {
        match self {
            Self::Claude => ("claude", vec![]),
            Self::OpenCode => ("opencode", vec![]),
            Self::Goose => ("goose", vec![]),
            Self::Shell => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
                let shell: &'static str = Box::leak(shell.into_boxed_str());
                (shell, vec![])
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalSplit {
    Vertical,
    Horizontal,
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
            terminal_panes: vec![TerminalPane::shell(&root_dir)],
            active_terminal: 0,
            terminal_ratio: 0.47,
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

    fn launch_in_active_terminal(&mut self, kind: AgentKind) {
        let (cmd, args) = kind.command();
        let cwd = self.root_dir.clone();
        if let Some(pane) = self.active_terminal_mut() {
            pane.title = kind.label().to_string();
            pane.spawn(cmd, &args, &cwd);
        }
    }

    fn split_terminal(&mut self, split: TerminalSplit) {
        self.terminal_split = split;
        self.terminal_panes
            .push(TerminalPane::shell(&self.root_dir));
        self.active_terminal = self.terminal_panes.len() - 1;
    }

    fn close_active_terminal(&mut self) {
        if self.terminal_panes.len() <= 1 {
            return;
        }
        self.terminal_panes.remove(self.active_terminal);
        if self.active_terminal >= self.terminal_panes.len() {
            self.active_terminal = self.terminal_panes.len() - 1;
        }
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
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("Terminal", |ui| {
                    if ui.button("New Shell").clicked() {
                        self.launch_in_active_terminal(AgentKind::Shell);
                        ui.close_menu();
                    }
                    if ui.button("Launch Claude").clicked() {
                        self.launch_in_active_terminal(AgentKind::Claude);
                        ui.close_menu();
                    }
                    if ui.button("Launch Goose").clicked() {
                        self.launch_in_active_terminal(AgentKind::Goose);
                        ui.close_menu();
                    }
                    if ui.button("Launch opencode").clicked() {
                        self.launch_in_active_terminal(AgentKind::OpenCode);
                        ui.close_menu();
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
                if let Some(ref msg) = self.status_message {
                    ui.separator();
                    ui.label(msg);
                }
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_width = ui.available_width();
            let left_width = (total_width * self.terminal_ratio).max(320.0);
            let right_width = (total_width - left_width - 8.0).max(420.0);
            let breadcrumb = self.breadcrumb();

            ui.horizontal(|ui| {
                ui.allocate_ui(egui::vec2(left_width, ui.available_height()), |ui| {
                    render_terminal_workspace(
                        ui,
                        &mut self.terminal_panes,
                        &mut self.active_terminal,
                        self.terminal_split,
                    );
                });

                ui.separator();

                ui.allocate_ui(egui::vec2(right_width, ui.available_height()), |ui| {
                    render_editor_workspace(
                        ui,
                        &mut self.buffers,
                        &mut self.active_buffer,
                        self.compare_buffer,
                        self.sidebar_visible,
                        self.file_tree.as_mut(),
                        self.wrap_lines,
                        self.editing,
                        &breadcrumb,
                    );
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
) {
    ui.horizontal(|ui| {
        ui.strong("Terminal");
        ui.separator();
        ui.label(match split {
            TerminalSplit::Vertical => "vertical split",
            TerminalSplit::Horizontal => "horizontal split",
        });
    });
    ui.separator();

    match split {
        TerminalSplit::Vertical => {
            ui.columns(panes.len().max(1), |columns| {
                for (idx, pane) in panes.iter_mut().enumerate() {
                    if let Some(column) = columns.get_mut(idx) {
                        render_terminal_pane(column, pane, idx, active_terminal);
                    }
                }
            });
        }
        TerminalSplit::Horizontal => {
            let pane_count = panes.len();
            for (idx, pane) in panes.iter_mut().enumerate() {
                render_terminal_pane(ui, pane, idx, active_terminal);
                if idx + 1 < pane_count {
                    ui.separator();
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
) {
    let selected = *active_terminal == idx;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui
                .selectable_label(selected, egui::RichText::new(&pane.title).strong())
                .clicked()
            {
                *active_terminal = idx;
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .max_height(420.0)
            .show(ui, |ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(12.0));
                ui.add(
                    egui::TextEdit::multiline(&mut pane.output)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(22)
                        .desired_width(f32::INFINITY)
                        .interactive(false),
                );
            });
        let response = ui.add(
            egui::TextEdit::singleline(&mut pane.input)
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .hint_text("Run a command in this pane"),
        );
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            pane.send_input();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn render_editor_workspace(
    ui: &mut egui::Ui,
    buffers: &mut [Buffer],
    active_buffer: &mut usize,
    compare_buffer: Option<usize>,
    sidebar_visible: bool,
    file_tree: Option<&mut FileTree>,
    wrap_lines: bool,
    editing: bool,
    breadcrumb: &str,
) {
    ui.horizontal(|ui| {
        let editor_total_width = ui.available_width();
        let sidebar_width = if sidebar_visible { 220.0 } else { 0.0 };
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
                let mut clicked_file = None;
                let mut toggle_dir = None;
                ui.allocate_ui(egui::vec2(sidebar_width, ui.available_height()), |ui| {
                    ui.heading("Files");
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
                                if *is_dir {
                                    toggle_dir = Some(path.clone());
                                } else {
                                    clicked_file = Some(path.clone());
                                }
                            }
                        }
                    });
                });

                if let Some(path) = toggle_dir {
                    if let Some(idx) = tree.entries.iter().position(|entry| entry.path == path) {
                        tree.toggle_expand(idx);
                    }
                }
                if let Some(path) = clicked_file {
                    if let Some(idx) = buffers
                        .iter()
                        .position(|buffer| buffer.path.as_deref() == Some(&path))
                    {
                        *active_buffer = idx;
                    } else if let Ok(buf) = Buffer::from_file(&path) {
                        buffers[*active_buffer] = buf;
                    }
                }
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
                            .selectable_label(*active_buffer == idx, format!("{title}{dirty}"))
                            .clicked()
                        {
                            *active_buffer = idx;
                        }
                    }
                });
                ui.separator();
                ui.label(egui::RichText::new(breadcrumb).small().monospace());
                ui.separator();

                if let Some(compare_idx) = compare_buffer {
                    ui.columns(2, |columns| {
                        render_editor_pane(
                            &mut columns[0],
                            &mut buffers[*active_buffer],
                            editing,
                            wrap_lines,
                            true,
                        );
                        if compare_idx < buffers.len() {
                            render_editor_pane(
                                &mut columns[1],
                                &mut buffers[compare_idx],
                                false,
                                wrap_lines,
                                false,
                            );
                        }
                    });
                } else {
                    render_editor_pane(ui, &mut buffers[*active_buffer], editing, wrap_lines, true);
                }
            },
        );
    });
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
        if editing {
            let response = egui::TextEdit::multiline(&mut text)
                .font(egui::TextStyle::Monospace)
                .desired_width(width)
                .desired_rows(30)
                .show(ui)
                .response;
            if response.changed() && text != buffer.content() {
                buffer.replace_content(&text);
            }
        } else {
            ui.add(
                egui::TextEdit::multiline(&mut text)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(width)
                    .desired_rows(30)
                    .interactive(false),
            );
        }
    });
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
            .with_inner_size([1600.0, 960.0])
            .with_title("edit"),
        ..Default::default()
    };

    eframe::run_native(
        "edit",
        options,
        Box::new(|_cc| Ok(Box::new(EditApp::new(paths)))),
    )
}
