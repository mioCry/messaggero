use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Metadata map carried in wire types.
///
/// Uses `String` values (rather than `serde_json::Value`) to remain compatible
/// with bincode, which does not support `deserialize_any`. Callers that need
/// richer structured values should JSON-encode them before insertion:
///
/// ```rust
/// # use messaggero_core::Metadata;
/// let mut meta = Metadata::new();
/// meta.insert("config".into(), serde_json::json!({"timeout": 30}).to_string());
/// ```
pub type Metadata = HashMap<String, String>;

// ---------------------------------------------------------------------------
// Agent Card (A2A v1.0 compatible)
// ---------------------------------------------------------------------------

/// Capability advertisement for an agent, following the A2A v1.0 schema.
///
/// An `AgentCard` is served at `GET /.well-known/agent.json` by the A2A HTTP
/// transport and is also used by the in-process `Discovery` registry so that
/// the `Router` can find local agents without any network round-trip.
///
/// Use the builder API instead of constructing this struct directly:
///
/// ```rust
/// # use messaggero_core::AgentCard;
/// let card = AgentCard::builder("translator")
///     .description("Translates text between languages")
///     .version("2.0.0")
///     .url("http://localhost:3001")
///     .skill("translate", "Translate", "Translate any text")
///     .streaming(false)
///     .build();
///
/// assert_eq!(card.name, "translator");
/// assert_eq!(card.skills.len(), 1);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    /// Unique human-readable name of this agent.
    pub name: String,
    /// Short description of what the agent does.
    pub description: String,
    /// Base URL where the agent's A2A HTTP endpoint is reachable.
    pub url: String,
    /// Semantic version string of the agent implementation.
    pub version: String,
    /// Optional feature flags (streaming, push notifications, …).
    #[serde(default)]
    pub capabilities: AgentCapabilities,
    /// List of skills (capabilities) this agent exposes.
    #[serde(default)]
    pub skills: Vec<AgentSkill>,
    /// Authentication requirements, if any.
    pub authentication: Option<AuthenticationInfo>,
    /// MIME types accepted as input (e.g. `["text/plain"]`).
    pub default_input_modes: Option<Vec<String>>,
    /// MIME types produced as output.
    pub default_output_modes: Option<Vec<String>>,
}

/// Optional capability flags advertised in the [`AgentCard`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming task responses.
    #[serde(default)]
    pub streaming: bool,
    /// Whether the agent can send push notifications for async tasks.
    #[serde(default)]
    pub push_notifications: bool,
    /// Whether the agent retains full state-transition history.
    #[serde(default)]
    pub state_transition_history: bool,
}

/// A single named skill advertised by an agent.
///
/// Skills are used by clients and routers to select the most appropriate agent
/// for a given task. They are informational only — routing is done by agent
/// name/endpoint, not skill id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    /// Unique identifier for this skill within the agent.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Optional explanation of what the skill does.
    pub description: Option<String>,
    /// Searchable tags (e.g. `["nlp", "translation"]`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Example prompts illustrating how to invoke the skill.
    pub examples: Option<Vec<String>>,
    /// MIME types accepted by this skill (overrides agent-level defaults).
    pub input_modes: Option<Vec<String>>,
    /// MIME types produced by this skill.
    pub output_modes: Option<Vec<String>>,
}

/// Authentication descriptor attached to an [`AgentCard`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthenticationInfo {
    /// Authentication scheme, e.g. `"Bearer"`, `"ApiKey"`, `"mTLS"`.
    pub auth_type: String,
    /// Opaque credentials or configuration hints (scheme-dependent).
    pub credentials: Option<String>,
}

// ---------------------------------------------------------------------------
// AgentCard builder
// ---------------------------------------------------------------------------

/// Fluent builder for [`AgentCard`].
///
/// Obtain one via [`AgentCard::builder`].
pub struct AgentCardBuilder {
    name: String,
    description: String,
    url: String,
    version: String,
    capabilities: AgentCapabilities,
    skills: Vec<AgentSkill>,
}

impl AgentCardBuilder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
            url: String::new(),
            version: "1.0.0".into(),
            capabilities: AgentCapabilities::default(),
            skills: Vec::new(),
        }
    }

    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    pub fn url(mut self, url: impl Into<String>) -> Self {
        self.url = url.into();
        self
    }

    pub fn version(mut self, ver: impl Into<String>) -> Self {
        self.version = ver.into();
        self
    }

    pub fn streaming(mut self, enabled: bool) -> Self {
        self.capabilities.streaming = enabled;
        self
    }

    pub fn skill(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
    ) -> Self {
        self.skills.push(AgentSkill {
            id: id.into(),
            name: name.into(),
            description: Some(description.into()),
            tags: Vec::new(),
            examples: None,
            input_modes: None,
            output_modes: None,
        });
        self
    }

    pub fn build(self) -> AgentCard {
        AgentCard {
            name: self.name,
            description: self.description,
            url: self.url,
            version: self.version,
            capabilities: self.capabilities,
            skills: self.skills,
            authentication: None,
            default_input_modes: None,
            default_output_modes: None,
        }
    }
}

impl AgentCard {
    pub fn builder(name: impl Into<String>) -> AgentCardBuilder {
        AgentCardBuilder::new(name)
    }
}

// ---------------------------------------------------------------------------
// Task lifecycle (A2A v1.0 compatible)
// ---------------------------------------------------------------------------

/// Lifecycle state of an A2A task.
///
/// The typical happy-path progression is:
/// `Submitted` → `Working` → `Completed`
///
/// An agent may also transition to `InputRequired` to request clarification,
/// or to `Failed`/`Canceled` for error paths.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskState {
    /// Task has been received but processing has not begun.
    Submitted,
    /// Task is actively being processed.
    Working,
    /// Agent paused and is waiting for additional user input.
    InputRequired,
    /// Task finished successfully.
    Completed,
    /// Task encountered an unrecoverable error.
    Failed,
    /// Task was explicitly canceled by the client.
    Canceled,
}

/// Snapshot of a task's current lifecycle state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskStatus {
    /// The lifecycle state of the task.
    pub state: TaskState,
    /// Optional message accompanying the state transition.
    pub message: Option<Message>,
    /// ISO-8601 timestamp of the last state change.
    pub timestamp: Option<String>,
}

/// Full task object as maintained by an agent (A2A v1.0 compatible).
///
/// In most cases you will interact with [`TaskRequest`] and [`TaskResponse`]
/// rather than `Task` directly. `Task` is the canonical stateful record used
/// when an agent needs to persist or inspect the full conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task identifier (UUID v4 by default).
    pub id: String,
    /// Optional session identifier for grouping related tasks.
    pub session_id: Option<String>,
    /// Current status of the task.
    pub status: TaskStatus,
    /// Full conversation history for this task.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Artifacts produced so far.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Application-defined metadata.
    pub metadata: Option<Metadata>,
}

impl Task {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id: None,
            status: TaskStatus {
                state: TaskState::Submitted,
                message: None,
                timestamp: None,
            },
            messages: Vec::new(),
            artifacts: Vec::new(),
            metadata: None,
        }
    }
}

impl Default for Task {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Message (A2A v1.0 compatible)
// ---------------------------------------------------------------------------

/// Sender role of a [`Message`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Message originated from a human user or upstream caller.
    User,
    /// Message originated from an AI agent.
    Agent,
}

/// A single message in a task conversation, composed of one or more [`Part`]s.
///
/// # Examples
///
/// ```rust
/// # use messaggero_core::{Message, Role};
/// let user_msg = Message::user("What is the weather in Rome?");
/// assert_eq!(user_msg.role, Role::User);
/// assert_eq!(user_msg.text_content(), Some("What is the weather in Rome?"));
///
/// let agent_msg = Message::agent("It is sunny and 24°C.");
/// assert_eq!(agent_msg.role, Role::Agent);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Who sent this message.
    pub role: Role,
    /// Content parts (text, file, structured data).
    pub parts: Vec<Part>,
    /// Application-defined metadata attached to this message.
    pub metadata: Option<Metadata>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            parts: vec![Part::text(text)],
            metadata: None,
        }
    }

    pub fn agent(text: impl Into<String>) -> Self {
        Self {
            role: Role::Agent,
            parts: vec![Part::text(text)],
            metadata: None,
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        self.parts.iter().find_map(|p| match p {
            Part::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
    }
}

// ---------------------------------------------------------------------------
// Part
// ---------------------------------------------------------------------------

/// Content part within a message.
///
/// Uses externally-tagged representation for bincode compatibility.
/// The A2A JSON-RPC layer converts to/from A2A's internally-tagged format
/// at the transport boundary.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Part {
    Text(TextPart),
    File(FilePart),
    Data(DataPart),
}

impl Part {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart { text: text.into() })
    }

    /// Wrap a JSON-serializable value as a data part.
    ///
    /// The value is JSON-encoded into a string for bincode compatibility.
    ///
    /// # Errors
    ///
    /// Returns a [`serde_json::Error`] if the value cannot be serialized.
    pub fn data(value: &impl serde::Serialize) -> Result<Self, serde_json::Error> {
        let json = serde_json::to_string(value)?;
        Ok(Self::Data(DataPart { json }))
    }

    /// Wrap a raw JSON string as a data part (no re-encoding).
    pub fn data_json(json: impl Into<String>) -> Self {
        Self::Data(DataPart { json: json.into() })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextPart {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilePart {
    pub name: Option<String>,
    pub mime_type: Option<String>,
    pub content: FileContent,
}

#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileContent {
    Bytes(String),
    Uri(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPart {
    /// JSON-encoded payload stored as a string for bincode compatibility.
    pub json: String,
}

// ---------------------------------------------------------------------------
// Artifact (A2A v1.0 compatible)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parts: Vec<Part>,
    #[serde(default)]
    pub index: u32,
    pub append: Option<bool>,
    pub last_chunk: Option<bool>,
}

// ---------------------------------------------------------------------------
// Task request / response (messaggero wire types)
// ---------------------------------------------------------------------------

/// Request sent to an agent to start or continue a task.
///
/// # Examples
///
/// ```rust
/// # use messaggero_core::{TaskRequest, Message};
/// let req = TaskRequest::new(Message::user("Summarise this document."))
///     .with_session("session-abc");
///
/// assert!(req.session_id.is_some());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest {
    /// Unique task ID (UUID v4 by default).
    pub id: String,
    /// Optional session grouping multiple tasks together.
    pub session_id: Option<String>,
    /// The message that initiates or continues the task.
    pub message: Message,
    /// Application-defined metadata forwarded to the agent.
    pub metadata: Option<Metadata>,
}

impl TaskRequest {
    pub fn new(message: Message) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            session_id: None,
            message,
            metadata: None,
        }
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
}

/// Response returned by an agent after processing a [`TaskRequest`].
///
/// Use the convenience constructors instead of building this struct manually:
///
/// ```rust
/// # use messaggero_core::{TaskResponse, Message};
/// let resp = TaskResponse::completed("task-1", Message::agent("Done!"));
/// assert!(resp.artifacts.is_empty());
///
/// let failed = TaskResponse::failed("task-2", "model timed out");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResponse {
    /// Echoes the task ID from the corresponding [`TaskRequest`].
    pub id: String,
    /// Optional session identifier.
    pub session_id: Option<String>,
    /// Final or intermediate status of the task.
    pub status: TaskStatus,
    /// Artifacts (files, data blobs) produced by the agent.
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    /// Application-defined metadata attached to the response.
    pub metadata: Option<Metadata>,
}

impl TaskResponse {
    pub fn completed(id: impl Into<String>, message: Message) -> Self {
        Self {
            id: id.into(),
            session_id: None,
            status: TaskStatus {
                state: TaskState::Completed,
                message: Some(message),
                timestamp: None,
            },
            artifacts: Vec::new(),
            metadata: None,
        }
    }

    pub fn failed(id: impl Into<String>, error_msg: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            session_id: None,
            status: TaskStatus {
                state: TaskState::Failed,
                message: Some(Message::agent(error_msg)),
                timestamp: None,
            },
            artifacts: Vec::new(),
            metadata: None,
        }
    }

    pub fn working(id: impl Into<String>, message: Message) -> Self {
        Self {
            id: id.into(),
            session_id: None,
            status: TaskStatus {
                state: TaskState::Working,
                message: Some(message),
                timestamp: None,
            },
            artifacts: Vec::new(),
            metadata: None,
        }
    }

    pub fn with_artifact(mut self, artifact: Artifact) -> Self {
        self.artifacts.push(artifact);
        self
    }
}
