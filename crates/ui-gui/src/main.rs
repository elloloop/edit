use core_buffer::Buffer;
use core_fs::FileTree;
use core_syntax::Highlighter;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

// =============================================================================
// Terminal Panel — runs an agent (Claude, opencode, goose) or a general shell
// =============================================================================

struct TerminalPanel {
    label: String,
    output: String,
    input: String,
    #[allow(dead_code)]
    process: Option<Child>,
    rx: Option<mpsc::Receiver<String>>,
}

impl TerminalPanel {
    fn new(label: &str) -> Self {
        Self {
            label: label.to_string(),
            output: String::new(),
            input: String::new(),
            process: None,
            rx: None,
        }
    }

    fn spawn(&mut self, cmd: &str, args: &[&str]) {
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);

        let cmd_owned = cmd.to_string();
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        std::thread::spawn(move || {
            let child = Command::new(&cmd_owned)
                .args(&args_owned)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();

            match child {
                Ok(mut c) => {
                    use std::io::Read;
                    if let Some(ref mut stdout) = c.stdout {
                        let mut buf = [0u8; 4096];
                        loop {
                            match stdout.read(&mut buf) {
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
                }
                Err(e) => {
                    let _ = tx.send(format!("Failed to start {cmd_owned}: {e}\n"));
                }
            }
        });
    }

    fn poll(&mut self) {
        if let Some(ref rx) = self.rx {
            while let Ok(text) = rx.try_recv() {
                self.output.push_str(&text);
                // Keep last 50k chars to prevent unbounded growth
                if self.output.len() > 50_000 {
                    let trim = self.output.len() - 40_000;
                    self.output = self.output[trim..].to_string();
                }
            }
        }
    }
}

// =============================================================================
// App State
// =============================================================================

enum AgentKind {
    Claude,
    OpenCode,
    Goose,
    Shell,
}

impl AgentKind {
    fn label(&self) -> &str {
        match self {
            Self::Claude => "Claude Code",
            Self::OpenCode => "opencode",
            Self::Goose => "Goose",
            Self::Shell => "Terminal",
        }
    }

    fn command(&self) -> (&str, Vec<&str>) {
        match self {
            Self::Claude => ("claude", vec![]),
            Self::OpenCode => ("opencode", vec![]),
            Self::Goose => ("goose", vec![]),
            Self::Shell => {
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
                // Leak to get &str — this is a one-time allocation for the app lifetime
                let shell: &str = Box::leak(shell.into_boxed_str());
                (shell, vec![])
            }
        }
    }
}

#[allow(dead_code)]
struct EditApp {
    buffers: Vec<Buffer>,
    active_buffer: usize,
    file_tree: Option<FileTree>,
    highlighters: HashMap<usize, Highlighter>,
    sidebar_visible: bool,
    root_dir: PathBuf,
    command_input: String,
    status_message: Option<String>,

    // Panels
    agent_panel: TerminalPanel,
    aux_terminal: Option<TerminalPanel>,
    show_aux_terminal: bool,

    // File watcher
    file_rx: Option<mpsc::Receiver<core_fs::FileEvent>>,
    #[allow(dead_code)]
    file_watcher: Option<core_fs::FileWatcherHandle>,
}

impl EditApp {
    fn new(path: Option<PathBuf>) -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();

        let (root_dir, initial_file) = match path {
            Some(p) => {
                let p = if p.is_absolute() { p } else { cwd.join(&p) };
                if p.is_dir() {
                    (p, None)
                } else {
                    let dir = p.parent().unwrap_or(&cwd).to_path_buf();
                    (dir, Some(p))
                }
            }
            None => (cwd, None),
        };

        let file_tree = FileTree::build(&root_dir).ok();

        let mut buffers = Vec::new();
        let mut highlighters = HashMap::new();

        if let Some(file_path) = initial_file {
            if let Ok(buf) = Buffer::from_file(&file_path) {
                let lang = buf.language.clone();
                buffers.push(buf);
                if let Some(mut hl) = Highlighter::new(&lang) {
                    let content = buffers[0].content();
                    hl.parse(&content);
                    highlighters.insert(0, hl);
                }
            }
        }

        if buffers.is_empty() {
            buffers.push(Buffer::from_string(""));
        }

        // Start file watcher
        let (tx, rx) = mpsc::channel();
        let watcher = core_fs::watch_directory(&root_dir, tx).ok();

        Self {
            buffers,
            active_buffer: 0,
            file_tree,
            highlighters,
            sidebar_visible: true,
            root_dir,
            command_input: String::new(),
            status_message: None,
            agent_panel: TerminalPanel::new("Agent"),
            aux_terminal: None,
            show_aux_terminal: false,
            file_rx: Some(rx),
            file_watcher: watcher,
        }
    }

    fn open_file(&mut self, path: &std::path::Path) {
        // Check if already open
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path) {
                self.active_buffer = i;
                return;
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
        }
    }

    fn process_file_events(&mut self) {
        let mut changed = vec![];
        if let Some(ref rx) = self.file_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    core_fs::FileEvent::Modified(p) | core_fs::FileEvent::Created(p) => {
                        changed.push(p);
                    }
                    _ => {}
                }
            }
        }

        for path in changed {
            let canon = path.canonicalize().unwrap_or(path);
            let idx = self.buffers.iter().position(|buf| {
                buf.path
                    .as_ref()
                    .map_or(false, |p| p.canonicalize().unwrap_or(p.clone()) == canon)
            });
            if let Some(idx) = idx {
                if !self.buffers[idx].dirty {
                    if let Ok(content) = std::fs::read_to_string(&canon) {
                        if content != self.buffers[idx].content() {
                            let name = self.buffers[idx].file_name();
                            self.buffers[idx].reload(&content);
                            if let Some(hl) = self.highlighters.get_mut(&idx) {
                                hl.parse(&content);
                            }
                            self.status_message = Some(format!("Reloaded: {name}"));
                        }
                    }
                }
            }
        }
    }

    fn launch_agent(&mut self, kind: AgentKind) {
        let label = kind.label().to_string();
        let (cmd, args) = kind.command();
        self.agent_panel = TerminalPanel::new(&label);
        self.agent_panel.spawn(cmd, &args);
    }

    fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.active_buffer]
    }
}

// =============================================================================
// egui Rendering
// =============================================================================

impl eframe::App for EditApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Poll terminals and file watcher
        self.agent_panel.poll();
        if let Some(ref mut aux) = self.aux_terminal {
            aux.poll();
        }
        self.process_file_events();

        // Request repaint every 100ms for live updates
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // ─── Top menu bar ───
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open...").clicked() {
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Agent", |ui| {
                    if ui.button("Launch Claude Code").clicked() {
                        self.launch_agent(AgentKind::Claude);
                        ui.close_menu();
                    }
                    if ui.button("Launch opencode").clicked() {
                        self.launch_agent(AgentKind::OpenCode);
                        ui.close_menu();
                    }
                    if ui.button("Launch Goose").clicked() {
                        self.launch_agent(AgentKind::Goose);
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Launch Shell").clicked() {
                        self.launch_agent(AgentKind::Shell);
                        ui.close_menu();
                    }
                });
                ui.menu_button("Terminal", |ui| {
                    let label = if self.show_aux_terminal {
                        "Hide Terminal"
                    } else {
                        "Show Terminal"
                    };
                    if ui.button(label).clicked() {
                        self.show_aux_terminal = !self.show_aux_terminal;
                        if self.show_aux_terminal && self.aux_terminal.is_none() {
                            let mut term = TerminalPanel::new("Terminal");
                            let shell =
                                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
                            term.spawn(&shell, &[]);
                            self.aux_terminal = Some(term);
                        }
                        ui.close_menu();
                    }
                });
                ui.menu_button("View", |ui| {
                    if ui
                        .button(if self.sidebar_visible {
                            "Hide Sidebar"
                        } else {
                            "Show Sidebar"
                        })
                        .clicked()
                    {
                        self.sidebar_visible = !self.sidebar_visible;
                        ui.close_menu();
                    }
                });
            });
        });

        // ─── Status bar at bottom ───
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.colored_label(egui::Color32::from_rgb(13, 147, 115), ">");
                let response = ui.text_edit_singleline(&mut self.command_input);
                if response.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter))
                    && !self.command_input.is_empty()
                {
                    self.status_message = Some(format!("Command: {}", self.command_input));
                    self.command_input.clear();
                }
                ui.separator();
                let buf = self.current_buffer();
                ui.label(format!(
                    "{}  {}  Ln {}, Col {}",
                    buf.file_name(),
                    buf.language,
                    buf.cursor_line + 1,
                    buf.cursor_col + 1
                ));
                if let Some(ref msg) = self.status_message {
                    ui.separator();
                    ui.label(msg);
                }
            });
        });

        // ─── Auxiliary terminal at bottom (above status) ───
        if self.show_aux_terminal {
            egui::TopBottomPanel::bottom("aux_terminal")
                .default_height(200.0)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.strong("Terminal");
                        if ui.small_button("x").clicked() {
                            self.show_aux_terminal = false;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            if let Some(ref aux) = self.aux_terminal {
                                ui.style_mut().override_font_id =
                                    Some(egui::FontId::monospace(12.0));
                                ui.label(&aux.output);
                            }
                        });
                });
        }

        // ─── Sidebar ───
        if self.sidebar_visible {
            // Collect entries before the closure to avoid borrow conflicts
            let sidebar_entries: Vec<(PathBuf, String, bool, usize)> = self
                .file_tree
                .as_ref()
                .map(|tree| {
                    tree.visible_entries()
                        .iter()
                        .map(|e| (e.path.clone(), e.name.clone(), e.is_dir, e.depth))
                        .collect()
                })
                .unwrap_or_default();

            let mut clicked_path = None;

            egui::SidePanel::left("sidebar")
                .default_width(200.0)
                .show(ctx, |ui| {
                    ui.heading("Files");
                    ui.separator();
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for (path, name, is_dir, depth) in &sidebar_entries {
                            let indent = "  ".repeat(*depth);
                            let icon = if *is_dir { "+" } else { " " };
                            let label = format!("{indent}{icon} {name}");
                            if ui.selectable_label(false, &label).clicked() && !is_dir {
                                clicked_path = Some(path.clone());
                            }
                        }
                    });
                });

            if let Some(path) = clicked_path {
                self.open_file(&path);
            }
        }

        // ─── Main area: Editor (left) + Agent panel (right) ───
        egui::CentralPanel::default().show(ctx, |ui| {
            let available = ui.available_size();
            let has_agent_output = !self.agent_panel.output.is_empty();

            // Split horizontally: editor | agent
            ui.horizontal_top(|ui| {
                let editor_width = if has_agent_output {
                    available.x * 0.5
                } else {
                    available.x
                };

                // ── Editor pane ──
                ui.allocate_ui(egui::vec2(editor_width, available.y), |ui| {
                    // Tab bar
                    ui.horizontal(|ui| {
                        for (i, buf) in self.buffers.iter().enumerate() {
                            let name = buf.file_name();
                            let dirty = if buf.dirty { " *" } else { "" };
                            if ui
                                .selectable_label(
                                    i == self.active_buffer,
                                    format!("{name}{dirty}"),
                                )
                                .clicked()
                            {
                                self.active_buffer = i;
                            }
                        }
                    });
                    ui.separator();

                    // Code viewer
                    let buf = self.current_buffer();
                    let content = buf.content();
                    let lines: Vec<&str> = content.lines().collect();

                    egui::ScrollArea::vertical().show(ui, |ui| {
                        ui.style_mut().override_font_id =
                            Some(egui::FontId::monospace(13.0));
                        for (i, line) in lines.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    egui::Color32::from_rgb(90, 90, 90),
                                    format!("{:>4} ", i + 1),
                                );
                                ui.label(*line);
                            });
                        }
                    });
                });

                // ── Agent pane (only if running) ──
                if has_agent_output {
                    ui.separator();
                    ui.allocate_ui(
                        egui::vec2(available.x - editor_width - 8.0, available.y),
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.strong(&self.agent_panel.label);
                            });
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    ui.style_mut().override_font_id =
                                        Some(egui::FontId::monospace(12.0));
                                    ui.label(&self.agent_panel.output);
                                });
                        },
                    );
                }
            });
        });
    }
}

// =============================================================================
// Entry point
// =============================================================================

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).map(PathBuf::from);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("edit"),
        ..Default::default()
    };

    eframe::run_native(
        "edit",
        options,
        Box::new(|_cc| Ok(Box::new(EditApp::new(path)))),
    )
}
