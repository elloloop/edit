use anyhow::{Context, Result};
use core_agent_protocol::{
    EditorAction, Envelope, HelloAck, Message, SessionId, WorkspaceQuery, WorkspaceSnapshot,
};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

#[derive(Debug)]
pub enum AgentBridgeEvent {
    Message(Envelope),
    Disconnected { session_id: Option<SessionId> },
    DecodeError(String),
}

pub struct AgentBridgeHandle {
    socket_path: PathBuf,
}

impl AgentBridgeHandle {
    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }
}

impl Drop for AgentBridgeHandle {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
pub fn start_agent_bridge(root_dir: PathBuf) -> Result<(AgentBridgeHandle, Receiver<AgentBridgeEvent>)> {
    use std::os::unix::net::UnixListener;

    let socket_path = std::env::temp_dir().join(format!("edit-agent-{}.sock", std::process::id()));
    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind bridge socket {}", socket_path.display()))?;
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let root_dir = root_dir.clone();
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        let _ = handle_client(root_dir, stream, tx);
                    });
                }
                Err(error) => {
                    let _ = tx.send(AgentBridgeEvent::DecodeError(error.to_string()));
                    break;
                }
            }
        }
    });

    Ok((AgentBridgeHandle { socket_path }, rx))
}

#[cfg(not(unix))]
pub fn start_agent_bridge(_root_dir: PathBuf) -> Result<(AgentBridgeHandle, Receiver<AgentBridgeEvent>)> {
    anyhow::bail!("agent bridge currently requires unix domain sockets")
}

#[cfg(unix)]
fn handle_client(
    root_dir: PathBuf,
    mut stream: std::os::unix::net::UnixStream,
    tx: Sender<AgentBridgeEvent>,
) -> Result<()> {
    let reader = stream.try_clone().context("clone bridge stream")?;
    let mut reader = BufReader::new(reader);
    let mut session_id = None;
    let mut line = String::new();

    loop {
        line.clear();
        let count = reader.read_line(&mut line).context("read bridge line")?;
        if count == 0 {
            let _ = tx.send(AgentBridgeEvent::Disconnected { session_id });
            return Ok(());
        }

        let envelope = match Envelope::from_json_line(&line) {
            Ok(envelope) => envelope,
            Err(error) => {
                let _ = tx.send(AgentBridgeEvent::DecodeError(error.to_string()));
                continue;
            }
        };
        session_id = Some(envelope.session_id.clone());

        match &envelope.message {
            Message::Hello(_) => {
                let reply = Envelope::new(
                    envelope.session_id.clone(),
                    Message::HelloAck(HelloAck {
                        editor_name: "edit".to_string(),
                        protocol_version: core_agent_protocol::PROTOCOL_VERSION,
                    }),
                );
                write_envelope(&mut stream, &reply)?;
            }
            Message::WorkspaceQuery(WorkspaceQuery::CurrentRoot) => {
                let reply = Envelope::new(
                    envelope.session_id.clone(),
                    Message::WorkspaceSnapshot(WorkspaceSnapshot::CurrentRoot {
                        path: root_dir.display().to_string(),
                    }),
                );
                write_envelope(&mut stream, &reply)?;
            }
            Message::WorkspaceQuery(WorkspaceQuery::VisibleFile) => {
                let reply = Envelope::new(
                    envelope.session_id.clone(),
                    Message::WorkspaceSnapshot(WorkspaceSnapshot::VisibleFile { path: None }),
                );
                write_envelope(&mut stream, &reply)?;
            }
            Message::EditorAction(EditorAction::OpenFile { .. })
            | Message::EditorAction(EditorAction::RevealPath { .. })
            | Message::EditorAction(EditorAction::ShowDiff { .. })
            | Message::SessionStatus(_)
            | Message::SubagentLifecycle(_)
            | Message::WorkspaceSnapshot(_)
            | Message::ActionResult(_)
            | Message::HelloAck(_) => {}
        }

        let _ = tx.send(AgentBridgeEvent::Message(envelope));
    }
}

#[cfg(unix)]
fn write_envelope(stream: &mut std::os::unix::net::UnixStream, envelope: &Envelope) -> Result<()> {
    let line = envelope.to_json_line()?;
    stream
        .write_all(line.as_bytes())
        .context("write bridge response")?;
    stream.flush().context("flush bridge response")?;
    Ok(())
}
