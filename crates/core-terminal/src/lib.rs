use anyhow::{Context, Result};
use fnug_vt100 as vt100;
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;

pub const DEFAULT_SCROLLBACK_LIMIT: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

impl TerminalSize {
    pub fn pty_size(self) -> PtySize {
        PtySize {
            rows: self.rows,
            cols: self.cols,
            pixel_width: 0,
            pixel_height: 0,
        }
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self {
            rows: 30,
            cols: 100,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSessionConfig {
    pub launcher: TerminalLauncher,
    pub cwd: PathBuf,
    pub size: TerminalSize,
    pub scrollback_limit: usize,
}

impl TerminalSessionConfig {
    pub fn new(launcher: TerminalLauncher, cwd: impl Into<PathBuf>) -> Self {
        Self {
            launcher,
            cwd: cwd.into(),
            size: TerminalSize::default(),
            scrollback_limit: DEFAULT_SCROLLBACK_LIMIT,
        }
    }

    pub fn with_size(mut self, size: TerminalSize) -> Self {
        self.size = size;
        self
    }

    pub fn with_scrollback_limit(mut self, scrollback_limit: usize) -> Self {
        self.scrollback_limit = scrollback_limit;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommand {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
}

impl TerminalCommand {
    pub fn new(
        label: impl Into<String>,
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            label: label.into(),
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalLauncher {
    Shell,
    Claude,
    Goose,
    OpenCode,
    Custom(TerminalCommand),
}

impl TerminalLauncher {
    pub fn label(&self) -> &str {
        match self {
            Self::Shell => "Shell",
            Self::Claude => "Claude",
            Self::Goose => "Goose",
            Self::OpenCode => "OpenCode",
            Self::Custom(command) => &command.label,
        }
    }

    pub fn command(&self) -> TerminalCommand {
        match self {
            Self::Shell => TerminalCommand::new(
                "Shell",
                std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string()),
                std::iter::empty::<String>(),
            ),
            Self::Claude => TerminalCommand::new("Claude", "claude", std::iter::empty::<String>()),
            Self::Goose => TerminalCommand::new("Goose", "goose", std::iter::empty::<String>()),
            Self::OpenCode => {
                TerminalCommand::new("OpenCode", "opencode", std::iter::empty::<String>())
            }
            Self::Custom(command) => command.clone(),
        }
    }
}

enum TerminalEvent {
    Output(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalSnapshot {
    pub text: String,
    pub size: TerminalSize,
    pub scrollback: usize,
    pub status: String,
    pub running: bool,
    pub application_cursor: bool,
}

struct TerminalSession {
    config: TerminalSessionConfig,
    parser: vt100::Parser,
    rx: Option<mpsc::Receiver<TerminalEvent>>,
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    scrollback: usize,
    running: bool,
    status: String,
}

impl TerminalSession {
    fn new(config: TerminalSessionConfig) -> Self {
        let size = config.size;
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, config.scrollback_limit),
            config,
            rx: None,
            writer: None,
            master: None,
            child: None,
            scrollback: 0,
            running: false,
            status: String::new(),
        }
    }

    fn spawn(&mut self) -> Result<()> {
        self.shutdown();
        self.parser = vt100::Parser::new(
            self.config.size.rows,
            self.config.size.cols,
            self.config.scrollback_limit,
        );
        self.scrollback = 0;

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(self.config.size.pty_size())
            .context("create pty pair")?;

        let command = self.config.launcher.command();
        let mut builder = CommandBuilder::new(&command.program);
        builder.args(command.args.iter().map(|arg| arg.as_str()));
        builder.cwd(&self.config.cwd);
        builder.env("TERM", "xterm-256color");

        let child = pair
            .slave
            .spawn_command(builder)
            .with_context(|| format!("spawn {}", command.label))?;
        let writer = pair.master.take_writer().context("open terminal writer")?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("open terminal reader")?;
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            let mut buf = [0_u8; 8192];
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

        self.rx = Some(rx);
        self.writer = Some(writer);
        self.master = Some(pair.master);
        self.child = Some(child);
        self.running = true;
        self.status = format!("Running {}", command.label);
        Ok(())
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    TerminalEvent::Output(bytes) => self.parser.process(&bytes),
                }
            }
        }

        if let Some(child) = self.child.as_mut() {
            if let Ok(Some(status)) = child.try_wait() {
                self.running = false;
                self.status = format!("Exited: {status}");
                self.parser
                    .process(format!("\r\n[{}]\r\n", self.status).as_bytes());
                self.child = None;
                self.writer = None;
                self.master = None;
            }
        }
    }

    fn resize(&mut self, size: TerminalSize) {
        if self.config.size == size {
            return;
        }

        self.config.size = size;
        self.parser.set_size(size.rows, size.cols);
        self.parser.set_scrollback(self.scrollback);
        if let Some(master) = self.master.as_ref() {
            let _ = master.resize(size.pty_size());
        }
    }

    fn send_input(&mut self, bytes: &[u8]) -> Result<()> {
        let writer = self
            .writer
            .as_mut()
            .context("terminal session is not writable")?;
        writer.write_all(bytes).context("write to terminal")?;
        writer.flush().context("flush terminal input")?;
        Ok(())
    }

    fn scroll(&mut self, delta: isize) {
        let len = self.parser.scrollback_len() as isize;
        let next = (self.scrollback as isize + delta).clamp(0, len);
        self.scrollback = next as usize;
        self.parser.set_scrollback(self.scrollback);
    }

    fn snapshot(&mut self) -> TerminalSnapshot {
        self.parser.set_scrollback(self.scrollback);
        let text = self.parser.screen().contents();
        TerminalSnapshot {
            text: if text.is_empty() {
                "\n".to_string()
            } else {
                text
            },
            size: self.config.size,
            scrollback: self.scrollback,
            status: self.status.clone(),
            running: self.running,
            application_cursor: self.parser.screen().application_cursor(),
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
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct TerminalRuntime {
    sessions: HashMap<u64, TerminalSession>,
}

impl TerminalRuntime {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub fn ensure_session(
        &mut self,
        session_id: u64,
        launcher: TerminalLauncher,
        cwd: impl Into<PathBuf>,
    ) -> Result<()> {
        self.ensure_session_with_config(session_id, TerminalSessionConfig::new(launcher, cwd))
    }

    pub fn ensure_session_with_config(
        &mut self,
        session_id: u64,
        config: TerminalSessionConfig,
    ) -> Result<()> {
        if self.sessions.contains_key(&session_id) {
            return Ok(());
        }

        let mut session = TerminalSession::new(config);
        session.spawn()?;
        self.sessions.insert(session_id, session);
        Ok(())
    }

    pub fn relaunch_session(
        &mut self,
        session_id: u64,
        launcher: TerminalLauncher,
        cwd: impl Into<PathBuf>,
    ) -> Result<()> {
        self.relaunch_session_with_config(session_id, TerminalSessionConfig::new(launcher, cwd))
    }

    pub fn relaunch_session_with_config(
        &mut self,
        session_id: u64,
        config: TerminalSessionConfig,
    ) -> Result<()> {
        let mut session = TerminalSession::new(config);
        session.spawn()?;
        self.sessions.insert(session_id, session);
        Ok(())
    }

    pub fn remove_session(&mut self, session_id: u64) {
        self.sessions.remove(&session_id);
    }

    pub fn has_session(&self, session_id: u64) -> bool {
        self.sessions.contains_key(&session_id)
    }

    pub fn session_ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.sessions.keys().copied()
    }

    pub fn poll_all(&mut self) {
        for session in self.sessions.values_mut() {
            session.poll();
        }
    }

    pub fn resize(&mut self, session_id: u64, size: TerminalSize) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.resize(size);
        }
    }

    pub fn send_input(&mut self, session_id: u64, bytes: &[u8]) -> Result<()> {
        self.sessions
            .get_mut(&session_id)
            .context("terminal session does not exist")?
            .send_input(bytes)
    }

    pub fn scroll(&mut self, session_id: u64, delta: isize) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.scroll(delta);
        }
    }

    pub fn snapshot(&mut self, session_id: u64) -> Option<TerminalSnapshot> {
        self.sessions
            .get_mut(&session_id)
            .map(TerminalSession::snapshot)
    }

    pub fn config(&self, session_id: u64) -> Option<&TerminalSessionConfig> {
        self.sessions
            .get(&session_id)
            .map(|session| &session.config)
    }
}

impl Default for TerminalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalCommand, TerminalLauncher, TerminalRuntime, TerminalSessionConfig, TerminalSize,
    };
    use std::path::Path;
    use std::time::{Duration, Instant};

    fn wait_for_snapshot(
        runtime: &mut TerminalRuntime,
        session_id: u64,
        predicate: impl Fn(&super::TerminalSnapshot) -> bool,
    ) -> super::TerminalSnapshot {
        let start = Instant::now();
        let mut snapshot = None;
        while start.elapsed() < Duration::from_secs(2) {
            runtime.poll_all();
            snapshot = runtime.snapshot(session_id);
            if snapshot.as_ref().is_some_and(&predicate) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        snapshot.expect("snapshot should exist")
    }

    #[test]
    fn launcher_resolves_builtin_commands() {
        let shell = TerminalLauncher::Shell.command();
        assert_eq!(shell.label, "Shell");
        assert!(!shell.program.is_empty());

        let claude = TerminalLauncher::Claude.command();
        assert_eq!(claude.label, "Claude");
        assert_eq!(claude.program, "claude");
    }

    #[test]
    fn runtime_can_spawn_custom_command_and_capture_output() {
        let mut runtime = TerminalRuntime::new();
        let launcher = TerminalLauncher::Custom(TerminalCommand::new(
            "Test Shell",
            "/bin/sh",
            ["-lc", "printf 'hello from terminal'; exit 0"],
        ));

        runtime
            .ensure_session(1, launcher, std::env::temp_dir())
            .expect("spawn test session");

        let snapshot = wait_for_snapshot(&mut runtime, 1, |snap| {
            snap.text.contains("hello from terminal")
        });
        assert!(snapshot.text.contains("hello from terminal"));
    }

    #[test]
    fn runtime_accepts_input_and_reports_exit_status() {
        let mut runtime = TerminalRuntime::new();
        let launcher = TerminalLauncher::Custom(TerminalCommand::new(
            "Echo Input",
            "/bin/sh",
            ["-lc", "read line; printf 'reply:%s' \"$line\""],
        ));

        runtime
            .ensure_session(3, launcher, std::env::temp_dir())
            .expect("spawn interactive session");
        runtime.send_input(3, b"typed text\n").expect("send input");

        let snapshot = wait_for_snapshot(&mut runtime, 3, |snap| {
            snap.text.contains("reply:typed text") && !snap.running
        });

        assert!(snapshot.text.contains("reply:typed text"));
        assert!(snapshot.status.starts_with("Exited:"));
    }

    #[test]
    fn resize_is_reflected_in_snapshot_and_config() {
        let mut runtime = TerminalRuntime::new();
        let config = TerminalSessionConfig::new(
            TerminalLauncher::Custom(TerminalCommand::new(
                "Long Running",
                "/bin/sh",
                ["-lc", "sleep 1"],
            )),
            std::env::temp_dir(),
        )
        .with_size(TerminalSize { rows: 12, cols: 40 });

        runtime
            .ensure_session_with_config(5, config)
            .expect("spawn sized session");

        let snapshot = runtime.snapshot(5).expect("snapshot should exist");
        assert_eq!(snapshot.size, TerminalSize { rows: 12, cols: 40 });

        runtime.resize(5, TerminalSize { rows: 24, cols: 80 });
        let resized = runtime.snapshot(5).expect("snapshot should exist");
        assert_eq!(resized.size, TerminalSize { rows: 24, cols: 80 });
        assert_eq!(
            runtime.config(5).expect("config should exist").size,
            TerminalSize { rows: 24, cols: 80 }
        );
    }

    #[test]
    fn relaunch_replaces_existing_session_configuration() {
        let mut runtime = TerminalRuntime::new();
        runtime
            .ensure_session(
                6,
                TerminalLauncher::Custom(TerminalCommand::new(
                    "First",
                    "/bin/sh",
                    ["-lc", "printf 'first'; exit 0"],
                )),
                std::env::temp_dir(),
            )
            .expect("spawn first session");

        let first = wait_for_snapshot(&mut runtime, 6, |snap| snap.text.contains("first"));
        assert!(first.text.contains("first"));

        runtime
            .relaunch_session_with_config(
                6,
                TerminalSessionConfig::new(
                    TerminalLauncher::Custom(TerminalCommand::new(
                        "Second",
                        "/bin/sh",
                        ["-lc", "printf 'second'; exit 0"],
                    )),
                    Path::new("/tmp"),
                ),
            )
            .expect("relaunch session");

        let second = wait_for_snapshot(&mut runtime, 6, |snap| snap.text.contains("second"));
        assert!(second.text.contains("second"));
        assert_eq!(
            runtime.config(6).expect("config should exist").cwd,
            Path::new("/tmp")
        );
    }

    #[test]
    fn removing_session_drops_it_from_runtime() {
        let mut runtime = TerminalRuntime::new();
        let launcher = TerminalLauncher::Custom(TerminalCommand::new(
            "Long Running",
            "/bin/sh",
            ["-lc", "sleep 1"],
        ));

        runtime
            .ensure_session(7, launcher, std::env::temp_dir())
            .expect("spawn long running session");
        assert!(runtime.has_session(7));

        runtime.remove_session(7);
        assert!(!runtime.has_session(7));
    }

    #[test]
    fn session_ids_report_all_registered_sessions() {
        let mut runtime = TerminalRuntime::new();
        runtime
            .ensure_session(
                11,
                TerminalLauncher::Custom(TerminalCommand::new(
                    "One",
                    "/bin/sh",
                    ["-lc", "sleep 1"],
                )),
                std::env::temp_dir(),
            )
            .expect("spawn first");
        runtime
            .ensure_session(
                12,
                TerminalLauncher::Custom(TerminalCommand::new(
                    "Two",
                    "/bin/sh",
                    ["-lc", "sleep 1"],
                )),
                std::env::temp_dir(),
            )
            .expect("spawn second");

        let mut ids: Vec<_> = runtime.session_ids().collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![11, 12]);
    }
}
