use crate::BackendError;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

const SESSION_BUFFER_CAP: usize = 2000;
const TERMINAL_BUFFER_MAX_EVENTS: usize = 12000;
const TERMINAL_BUFFER_MAX_BYTES: usize = 3 * 1024 * 1024;
const REPLAYABLE_EVENTS: &[&str] = &[
    "chat:sending",
    "chat:chunk",
    "chat:tool_use",
    "chat:tool_block",
    "chat:tool_result",
    "chat:thinking",
    "chat:permission_denied",
    "chat:codex_command_approval_request",
    "chat:codex_permission_request",
    "chat:codex_user_input_request",
    "chat:codex_mcp_elicitation_request",
    "chat:codex_dynamic_tool_call_request",
    "chat:done",
    "chat:cancelled",
    "chat:error",
];
const TERMINAL_REPLAYABLE_EVENTS: &[&str] = &["terminal:output", "terminal:started"];
type ReplayEvent = (u64, Arc<str>);
type SessionReplayBuffers = HashMap<String, VecDeque<ReplayEvent>>;

pub trait EventSink: Send + Sync {
    fn emit_json(&self, event: &str, payload: Value) -> Result<(), BackendError>;
}

#[derive(Clone, Debug)]
pub struct WsEvent {
    pub json: Arc<str>,
    pub seq: u64,
}

#[derive(Debug, Default)]
struct TerminalReplayBuffer {
    events: VecDeque<(u64, Arc<str>)>,
    bytes: usize,
}

impl TerminalReplayBuffer {
    fn push(&mut self, seq: u64, json: Arc<str>) {
        let event_bytes = json.len();
        if event_bytes > TERMINAL_BUFFER_MAX_BYTES {
            self.events.clear();
            self.bytes = 0;
            return;
        }
        while !self.events.is_empty()
            && (self.events.len() >= TERMINAL_BUFFER_MAX_EVENTS
                || self.bytes + event_bytes > TERMINAL_BUFFER_MAX_BYTES)
        {
            if let Some((_, old)) = self.events.pop_front() {
                self.bytes = self.bytes.saturating_sub(old.len());
            }
        }
        self.bytes += event_bytes;
        self.events.push_back((seq, json));
    }
}

pub struct WsBroadcaster {
    tx: broadcast::Sender<WsEvent>,
    active: AtomicBool,
    next_seq: AtomicU64,
    session_buffers: Mutex<SessionReplayBuffers>,
    terminal_buffers: Mutex<HashMap<String, TerminalReplayBuffer>>,
}

impl Default for WsBroadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl WsBroadcaster {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(8192);
        Self {
            tx,
            active: AtomicBool::new(false),
            next_seq: AtomicU64::new(1),
            session_buffers: Mutex::new(HashMap::new()),
            terminal_buffers: Mutex::new(HashMap::new()),
        }
    }

    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
        if !active {
            self.session_buffers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
            self.terminal_buffers
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clear();
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.tx.subscribe()
    }

    pub fn broadcast<S: Serialize>(&self, event: &str, payload: &S) -> Result<(), BackendError> {
        if !self.is_active() {
            return Ok(());
        }
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let json: Arc<str> = Arc::from(serde_json::to_string(&serde_json::json!({
            "type": "event",
            "event": event,
            "payload": payload,
            "seq": seq,
        }))?);
        let value = serde_json::to_value(payload)?;

        if REPLAYABLE_EVENTS.contains(&event) {
            if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                let mut buffers = self
                    .session_buffers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                let buffer = buffers
                    .entry(session_id.to_string())
                    .or_insert_with(|| VecDeque::with_capacity(SESSION_BUFFER_CAP));
                if buffer.len() >= SESSION_BUFFER_CAP {
                    buffer.pop_front();
                }
                buffer.push_back((seq, json.clone()));
            }
        }
        if matches!(event, "chat:done" | "chat:cancelled") {
            if let Some(session_id) = value.get("session_id").and_then(Value::as_str) {
                self.session_buffers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(session_id);
            }
        }
        if TERMINAL_REPLAYABLE_EVENTS.contains(&event) {
            if let Some(terminal_id) = value.get("terminal_id").and_then(Value::as_str) {
                self.terminal_buffers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .entry(terminal_id.to_string())
                    .or_default()
                    .push(seq, json.clone());
            }
        }
        if event == "terminal:stopped" {
            if let Some(terminal_id) = value.get("terminal_id").and_then(Value::as_str) {
                self.terminal_buffers
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(terminal_id);
            }
        }
        let _ = self.tx.send(WsEvent { json, seq });
        Ok(())
    }

    pub fn replay_events(&self, session_id: &str, after_seq: u64) -> Vec<WsEvent> {
        self.session_buffers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(session_id)
            .into_iter()
            .flatten()
            .filter(|(seq, _)| *seq > after_seq)
            .map(|(seq, json)| WsEvent {
                json: json.clone(),
                seq: *seq,
            })
            .collect()
    }

    pub fn replay_terminal_events(&self, terminal_id: &str, after_seq: u64) -> Vec<WsEvent> {
        self.terminal_buffers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(terminal_id)
            .into_iter()
            .flat_map(|buffer| buffer.events.iter())
            .filter(|(seq, _)| *seq > after_seq)
            .map(|(seq, json)| WsEvent {
                json: json.clone(),
                seq: *seq,
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct ServerEventSink {
    broadcaster: Arc<WsBroadcaster>,
}

impl ServerEventSink {
    pub fn new(broadcaster: Arc<WsBroadcaster>) -> Self {
        Self { broadcaster }
    }
}

impl EventSink for ServerEventSink {
    fn emit_json(&self, event: &str, payload: Value) -> Result<(), BackendError> {
        self.broadcaster.broadcast(event, &payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_broadcaster_does_not_buffer() {
        let broadcaster = WsBroadcaster::new();
        broadcaster
            .broadcast("chat:chunk", &serde_json::json!({"session_id": "s1"}))
            .unwrap();
        assert!(broadcaster.replay_events("s1", 0).is_empty());
    }

    #[test]
    fn chat_and_terminal_events_are_replayed_in_order() {
        let broadcaster = WsBroadcaster::new();
        broadcaster.set_active(true);
        broadcaster
            .broadcast(
                "chat:chunk",
                &serde_json::json!({"session_id": "s1", "content": "a"}),
            )
            .unwrap();
        broadcaster
            .broadcast(
                "chat:chunk",
                &serde_json::json!({"session_id": "s1", "content": "b"}),
            )
            .unwrap();
        broadcaster
            .broadcast(
                "terminal:output",
                &serde_json::json!({"terminal_id": "t1", "data": "x"}),
            )
            .unwrap();

        let chat = broadcaster.replay_events("s1", 0);
        assert_eq!(chat.len(), 2);
        assert!(chat[0].seq < chat[1].seq);
        assert_eq!(broadcaster.replay_terminal_events("t1", 0).len(), 1);
    }

    #[test]
    fn completion_clears_chat_replay_buffer() {
        let broadcaster = WsBroadcaster::new();
        broadcaster.set_active(true);
        broadcaster
            .broadcast("chat:chunk", &serde_json::json!({"session_id": "s1"}))
            .unwrap();
        broadcaster
            .broadcast("chat:done", &serde_json::json!({"session_id": "s1"}))
            .unwrap();
        assert!(broadcaster.replay_events("s1", 0).is_empty());
    }
}
