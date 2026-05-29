use anyhow::{bail, Context, Result};
use core_agent_protocol::{
    Capability, EditorAction, Envelope, GuidedEditorAction, Hello, Message, SessionId,
    SessionStatus, SessionStatusKind, SubagentId, SubagentLifecycle, SubagentLifecycleKind,
    TextSelection, WorkspaceQuery,
};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunArgs {
    pub session_id: SessionId,
    pub bridge: Option<PathBuf>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Run(RunArgs),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentHandle {
    pub id: SubagentId,
    pub label: String,
}

#[derive(Debug, Clone)]
pub struct AgentRuntime {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub subagents: Vec<SubagentHandle>,
    next_subagent: u64,
}

impl AgentRuntime {
    pub fn new(args: RunArgs) -> Self {
        Self {
            session_id: args.session_id,
            cwd: args.cwd,
            subagents: Vec::new(),
            next_subagent: 1,
        }
    }

    pub fn spawn_subagent(&mut self, label: impl Into<String>) -> SubagentHandle {
        let label = label.into();
        let id = SubagentId::new(format!("subagent-{}", self.next_subagent))
            .expect("generated subagent id should be valid");
        self.next_subagent += 1;
        let handle = SubagentHandle { id, label };
        self.subagents.push(handle.clone());
        handle
    }
}

pub fn parse_args(args: &[String]) -> Result<Command> {
    if args.is_empty() {
        bail!("usage: edit-agent run [--bridge PATH] [--session-id ID] [--cwd PATH]");
    }

    match args[0].as_str() {
        "run" => parse_run_args(&args[1..]).map(Command::Run),
        other => bail!("unknown command: {other}"),
    }
}

fn parse_run_args(args: &[String]) -> Result<RunArgs> {
    let mut bridge = None;
    let mut session_id = None;
    let mut cwd = std::env::current_dir().context("resolve current directory")?;

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--bridge" => {
                i += 1;
                bridge = Some(PathBuf::from(
                    args.get(i).context("missing value for --bridge")?,
                ));
            }
            "--session-id" => {
                i += 1;
                session_id = Some(SessionId::new(
                    args.get(i).context("missing value for --session-id")?.clone(),
                )?);
            }
            "--cwd" => {
                i += 1;
                cwd = PathBuf::from(args.get(i).context("missing value for --cwd")?);
            }
            other => bail!("unknown run option: {other}"),
        }
        i += 1;
    }

    Ok(RunArgs {
        session_id: session_id.unwrap_or(SessionId::new("default-session")?),
        bridge,
        cwd,
    })
}

pub fn run(args: RunArgs) -> Result<()> {
    let mut runtime = AgentRuntime::new(args.clone());
    println!(
        "edit-agent session {} in {}",
        runtime.session_id,
        runtime.cwd.display()
    );

    #[cfg(unix)]
    if let Some(bridge_path) = &args.bridge {
        use std::os::unix::net::UnixStream;

        let stream = UnixStream::connect(bridge_path)
            .with_context(|| format!("connect bridge {}", bridge_path.display()))?;
        let reader = stream.try_clone().context("clone bridge stream")?;
        let mut transport = AgentTransport {
            reader: BufReader::new(reader),
            writer: stream,
        };

        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::Hello(Hello {
                runtime_name: "edit-agent".to_string(),
                capabilities: vec![
                    Capability::CurrentRoot,
                    Capability::OpenFile,
                    Capability::RevealPath,
                    Capability::ShowDiff,
                    Capability::FocusEditor,
                    Capability::SelectRange,
                    Capability::SubagentStatus,
                ],
            }),
        ))?;
        if let Some(reply) = transport.read()? {
            println!("bridge: {}", reply);
        }

        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::SessionStatus(SessionStatus {
                kind: SessionStatusKind::Starting,
                detail: Some("edit-agent booted".to_string()),
            }),
        ))?;
        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::WorkspaceQuery(WorkspaceQuery::CurrentRoot),
        ))?;
        if let Some(reply) = transport.read()? {
            println!("bridge: {}", reply);
        }

        let subagent = runtime.spawn_subagent("bootstrap");
        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::SubagentLifecycle(SubagentLifecycle {
                subagent_id: subagent.id.clone(),
                label: subagent.label.clone(),
                kind: SubagentLifecycleKind::Started,
            }),
        ))?;
        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::EditorAction(EditorAction::RevealPath {
                path: "Cargo.toml".to_string(),
            }),
        ))?;
        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::SubagentLifecycle(SubagentLifecycle {
                subagent_id: subagent.id,
                label: subagent.label,
                kind: SubagentLifecycleKind::Finished,
            }),
        ))?;
        for envelope in guided_editor_demo_envelopes(&runtime) {
            transport.send(envelope)?;
        }
        transport.send(Envelope::new(
            runtime.session_id.clone(),
            Message::SessionStatus(SessionStatus {
                kind: SessionStatusKind::Running,
                detail: Some("bridge online".to_string()),
            }),
        ))?;
    }

    println!("commands: status | subagent <name> | open <path> | reveal <path> | diff <path> | quit");
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("edit-agent> ");
        std::io::stdout().flush().ok();
        line.clear();
        if stdin.read_line(&mut line)? == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, "quit" | "exit") {
            break;
        }
        println!("received: {trimmed}");
    }

    Ok(())
}

#[cfg(unix)]
struct AgentTransport {
    reader: BufReader<std::os::unix::net::UnixStream>,
    writer: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl AgentTransport {
    fn send(&mut self, envelope: Envelope) -> Result<()> {
        let line = envelope.to_json_line()?;
        self.writer
            .write_all(line.as_bytes())
            .context("write bridge envelope")?;
        self.writer.flush().context("flush bridge envelope")?;
        Ok(())
    }

    fn read(&mut self) -> Result<Option<String>> {
        let mut line = String::new();
        let count = self.reader.read_line(&mut line).context("read bridge reply")?;
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim().to_string()))
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Run(args) => write!(f, "run {}", args.session_id),
        }
    }
}

fn guided_editor_demo_envelopes(runtime: &AgentRuntime) -> Vec<Envelope> {
    vec![
        Envelope::new(
            runtime.session_id.clone(),
            Message::SessionStatus(SessionStatus {
                kind: SessionStatusKind::Running,
                detail: Some("focus editor".to_string()),
            }),
        )
        .with_guidance(GuidedEditorAction::FocusEditor),
        Envelope::new(
            runtime.session_id.clone(),
            Message::SessionStatus(SessionStatus {
                kind: SessionStatusKind::Running,
                detail: Some("jump to Cargo.toml:1".to_string()),
            }),
        )
        .with_guidance(GuidedEditorAction::SelectText {
            path: "Cargo.toml".to_string(),
            selection: TextSelection::jump_to_line(1),
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_run_command_with_explicit_values() {
        let args = vec![
            "run".to_string(),
            "--bridge".to_string(),
            "/tmp/edit.sock".to_string(),
            "--session-id".to_string(),
            "pane-7".to_string(),
            "--cwd".to_string(),
            "/tmp/work".to_string(),
        ];
        let command = parse_args(&args).unwrap();
        assert_eq!(
            command,
            Command::Run(RunArgs {
                session_id: SessionId::new("pane-7").unwrap(),
                bridge: Some(PathBuf::from("/tmp/edit.sock")),
                cwd: PathBuf::from("/tmp/work"),
            })
        );
    }

    #[test]
    fn runtime_generates_stable_subagent_ids() {
        let args = RunArgs {
            session_id: SessionId::new("pane-1").unwrap(),
            bridge: None,
            cwd: PathBuf::from("."),
        };
        let mut runtime = AgentRuntime::new(args);
        let first = runtime.spawn_subagent("plan");
        let second = runtime.spawn_subagent("verify");

        assert_eq!(first.id.as_str(), "subagent-1");
        assert_eq!(second.id.as_str(), "subagent-2");
        assert_eq!(runtime.subagents.len(), 2);
    }

    #[test]
    fn parse_requires_known_commands() {
        let error = parse_args(&["unknown".to_string()]).unwrap_err();
        assert!(error.to_string().contains("unknown command"));
    }

    #[test]
    fn guided_editor_demo_envelopes_include_focus_and_selection() {
        let runtime = AgentRuntime::new(RunArgs {
            session_id: SessionId::new("pane-1").unwrap(),
            bridge: None,
            cwd: PathBuf::from("."),
        });

        let envelopes = guided_editor_demo_envelopes(&runtime);
        assert_eq!(envelopes.len(), 2);
        assert!(matches!(
            envelopes[0].guidance,
            Some(GuidedEditorAction::FocusEditor)
        ));
        assert!(matches!(
            envelopes[1].guidance,
            Some(GuidedEditorAction::SelectText { ref path, ref selection })
                if path == "Cargo.toml" && selection.end.is_none() && selection.start.line == 1
        ));
    }
}
