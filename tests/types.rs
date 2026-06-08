//! Unit-style tests for core wire types (no transport required).
//! Kept in the integration test layer so they exercise the public API surface.

use messaggero::{AgentCard, Artifact, Message, Part, Role, TaskRequest, TaskResponse, TaskState};
use serde::{Deserialize, Serialize};

// ── AgentCard ────────────────────────────────────────────────────────────────

#[test]
fn test_agent_card_builder_defaults() {
    let card = AgentCard::builder("my-agent").build();
    assert_eq!(card.name, "my-agent");
    assert_eq!(card.version, "1.0.0");
    assert!(!card.capabilities.streaming);
    assert!(card.skills.is_empty());
}

#[test]
fn test_agent_card_builder_full() {
    let card = AgentCard::builder("bot")
        .description("A helpful bot")
        .version("2.1.0")
        .url("http://example.com")
        .streaming(true)
        .skill("chat", "Chat", "Have a conversation")
        .build();

    assert_eq!(card.description, "A helpful bot");
    assert_eq!(card.version, "2.1.0");
    assert_eq!(card.url, "http://example.com");
    assert!(card.capabilities.streaming);
    assert_eq!(card.skills.len(), 1);
    assert_eq!(card.skills[0].id, "chat");
}

// ── Message ──────────────────────────────────────────────────────────────────

#[test]
fn test_message_user() {
    let msg = Message::user("hello");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.text_content(), Some("hello"));
}

#[test]
fn test_message_agent() {
    let msg = Message::agent("response");
    assert_eq!(msg.role, Role::Agent);
    assert_eq!(msg.text_content(), Some("response"));
}

#[test]
fn test_message_no_text_part() {
    use messaggero::FilePart;
    let msg = Message {
        role: Role::Agent,
        parts: vec![Part::File(FilePart {
            name: Some("doc.pdf".into()),
            mime_type: Some("application/pdf".into()),
            content: messaggero::FileContent::Uri("https://example.com/doc.pdf".into()),
        })],
        metadata: None,
    };
    assert_eq!(msg.text_content(), None);
}

// ── Part ─────────────────────────────────────────────────────────────────────

#[test]
fn test_part_data_roundtrip() {
    #[derive(Serialize, Deserialize, PartialEq, Debug)]
    struct Payload {
        value: u32,
    }

    let payload = Payload { value: 42 };
    let part = Part::data(&payload).unwrap();

    if let Part::Data(dp) = part {
        let decoded: Payload = serde_json::from_str(&dp.json).unwrap();
        assert_eq!(decoded, payload);
    } else {
        panic!("expected Part::Data");
    }
}

// ── TaskRequest ───────────────────────────────────────────────────────────────

#[test]
fn test_task_request_new_generates_uuid() {
    let r1 = TaskRequest::new(Message::user("a"));
    let r2 = TaskRequest::new(Message::user("b"));
    assert_ne!(r1.id, r2.id, "each request should have a unique ID");
}

#[test]
fn test_task_request_with_id() {
    let req = TaskRequest::new(Message::user("x")).with_id("fixed-id");
    assert_eq!(req.id, "fixed-id");
}

#[test]
fn test_task_request_with_session() {
    let req = TaskRequest::new(Message::user("x")).with_session("sess-99");
    assert_eq!(req.session_id, Some("sess-99".into()));
}

// ── TaskResponse ──────────────────────────────────────────────────────────────

#[test]
fn test_task_response_completed() {
    let resp = TaskResponse::completed("t1", Message::agent("done"));
    assert_eq!(resp.id, "t1");
    assert_eq!(resp.status.state, TaskState::Completed);
    assert!(resp.artifacts.is_empty());
}

#[test]
fn test_task_response_failed() {
    let resp = TaskResponse::failed("t2", "oops");
    assert_eq!(resp.status.state, TaskState::Failed);
    let msg = resp.status.message.unwrap();
    assert_eq!(msg.text_content(), Some("oops"));
}

#[test]
fn test_task_response_with_artifact() {
    let artifact = Artifact {
        name: Some("output.txt".into()),
        description: None,
        parts: vec![Part::text("result")],
        index: 0,
        append: None,
        last_chunk: Some(true),
    };
    let resp = TaskResponse::completed("t3", Message::agent("done")).with_artifact(artifact);
    assert_eq!(resp.artifacts.len(), 1);
    assert_eq!(resp.artifacts[0].name.as_deref(), Some("output.txt"));
}

// ── Serialization round-trip ──────────────────────────────────────────────────

#[test]
fn test_task_response_json_roundtrip() {
    let original = TaskResponse::completed("rt-1", Message::agent("hi"));
    let json = serde_json::to_string(&original).unwrap();
    let decoded: TaskResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, original.id);
    assert_eq!(decoded.status.state, original.status.state);
}
