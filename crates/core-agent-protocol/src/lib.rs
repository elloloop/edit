use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fmt;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProtocolError::InvalidId("session_id"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubagentId(String);

impl SubagentId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProtocolError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ProtocolError::InvalidId("subagent_id"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SubagentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    OpenFile,
    RevealPath,
    ShowDiff,
    FocusEditor,
    SelectRange,
    CurrentRoot,
    VisibleFile,
    SubagentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub runtime_name: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub editor_name: String,
    pub protocol_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatusKind {
    Starting,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub kind: SessionStatusKind,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubagentLifecycleKind {
    Started,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentLifecycle {
    pub subagent_id: SubagentId,
    pub label: String,
    pub kind: SubagentLifecycleKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceQuery {
    CurrentRoot,
    VisibleFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceSnapshot {
    CurrentRoot { path: String },
    VisibleFile { path: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextAnchor {
    pub line: u32,
    pub column: Option<u32>,
}

impl TextAnchor {
    pub fn new(line: u32, column: Option<u32>) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSelection {
    pub start: TextAnchor,
    pub end: Option<TextAnchor>,
}

impl TextSelection {
    pub fn jump_to_line(line: u32) -> Self {
        Self {
            start: TextAnchor::new(line, None),
            end: None,
        }
    }

    pub fn range(
        start_line: u32,
        start_column: Option<u32>,
        end_line: u32,
        end_column: Option<u32>,
    ) -> Self {
        Self {
            start: TextAnchor::new(start_line, start_column),
            end: Some(TextAnchor::new(end_line, end_column)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuidedEditorAction {
    FocusEditor,
    SelectText { path: String, selection: TextSelection },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorAction {
    OpenFile { path: String },
    RevealPath { path: String },
    ShowDiff { path: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub ok: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Message {
    Hello(Hello),
    HelloAck(HelloAck),
    SessionStatus(SessionStatus),
    SubagentLifecycle(SubagentLifecycle),
    WorkspaceQuery(WorkspaceQuery),
    WorkspaceSnapshot(WorkspaceSnapshot),
    EditorAction(EditorAction),
    ActionResult(ActionResult),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub session_id: SessionId,
    pub message: Message,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guidance: Option<GuidedEditorAction>,
}

impl Envelope {
    pub fn new(session_id: SessionId, message: Message) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            session_id,
            message,
            guidance: None,
        }
    }

    pub fn with_guidance(mut self, guidance: GuidedEditorAction) -> Self {
        self.guidance = Some(guidance);
        self
    }

    pub fn to_json_line(&self) -> Result<String, serde_json::Error> {
        let json = serde_json::to_string(self)?;
        Ok(format!("{json}\n"))
    }

    pub fn from_json_line(line: &str) -> Result<Self, ProtocolError> {
        let envelope: Self = serde_json::from_str(line.trim_end())?;
        if envelope.version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(envelope.version));
        }
        Ok(envelope)
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    InvalidId(&'static str),
    UnsupportedVersion(u32),
    Json(serde_json::Error),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(field) => write!(f, "invalid {field}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported protocol version {version}")
            }
            Self::Json(error) => error.fmt(f),
        }
    }
}

impl Error for ProtocolError {}

impl From<serde_json::Error> for ProtocolError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_round_trips_through_json_lines() {
        let envelope = Envelope::new(
            SessionId::new("pane-1").unwrap(),
            Message::Hello(Hello {
                runtime_name: "edit-agent".to_string(),
                capabilities: vec![Capability::CurrentRoot, Capability::OpenFile],
            }),
        );

        let json = envelope.to_json_line().unwrap();
        let decoded = Envelope::from_json_line(&json).unwrap();
        assert_eq!(decoded, envelope);
    }

    #[test]
    fn unsupported_versions_are_rejected() {
        let json = r#"{"version":999,"session_id":"pane-1","message":{"WorkspaceQuery":"CurrentRoot"}}"#;
        let error = Envelope::from_json_line(json).unwrap_err();
        assert!(matches!(error, ProtocolError::UnsupportedVersion(999)));
    }

    #[test]
    fn ids_must_not_be_empty() {
        assert!(SessionId::new("").is_err());
        assert!(SubagentId::new("   ").is_err());
    }

    #[test]
    fn envelope_round_trips_with_guidance() {
        let envelope = Envelope::new(
            SessionId::new("pane-9").unwrap(),
            Message::SessionStatus(SessionStatus {
                kind: SessionStatusKind::Running,
                detail: Some("guidance".to_string()),
            }),
        )
        .with_guidance(GuidedEditorAction::SelectText {
            path: "src/main.rs".to_string(),
            selection: TextSelection::range(12, Some(3), 14, Some(8)),
        });

        let json = envelope.to_json_line().unwrap();
        let decoded = Envelope::from_json_line(&json).unwrap();
        assert_eq!(decoded, envelope);
        assert!(matches!(
            decoded.guidance,
            Some(GuidedEditorAction::SelectText { ref path, ref selection })
                if path == "src/main.rs"
                    && selection.start.line == 12
                    && selection.end.as_ref().map(|anchor| anchor.line) == Some(14)
        ));
    }

    #[test]
    fn jump_to_line_uses_single_selection_shape() {
        let selection = TextSelection::jump_to_line(27);
        assert_eq!(selection.start.line, 27);
        assert_eq!(selection.start.column, None);
        assert_eq!(selection.end, None);
    }
}
