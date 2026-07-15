use crate::{BackendError, BackendErrorCode, PersistenceService};
use serde_json::{Map, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone)]
pub struct SessionService {
    persistence: Arc<PersistenceService>,
}

impl SessionService {
    pub fn new(persistence: Arc<PersistenceService>) -> Self {
        Self { persistence }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        worktree_id: &str,
        name: Option<&str>,
        backend: Option<&str>,
        primary_surface: Option<&str>,
        terminal_command: Option<&str>,
        terminal_command_args: Option<&[String]>,
        terminal_label: Option<&str>,
        native_session_id: Option<&str>,
    ) -> Result<Value, BackendError> {
        if native_session_id.is_some() && primary_surface != Some("terminal") {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Native session IDs are only valid for terminal sessions",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let created_at = now();
        let backend = backend.unwrap_or("claude").to_string();
        let (session_name, order) = self.persistence.update_session_index(
            worktree_id,
            empty_index(worktree_id),
            |index| {
                let object = object_mut(index)?;
                let sessions = object
                    .entry("sessions")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| invalid_data("sessions"))?;
                let order = u32::try_from(sessions.len()).unwrap_or(u32::MAX);
                let session_name = name
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .unwrap_or_else(|| format!("Session {}", sessions.len() + 1));
                sessions.push(serde_json::json!({
                    "id": id,
                    "name": session_name,
                    "order": order,
                    "message_count": 0,
                }));
                object.insert("active_session_id".to_string(), Value::String(id.clone()));
                Ok((session_name, order))
            },
        )?;
        let mut metadata = serde_json::json!({
            "id": id,
            "worktree_id": worktree_id,
            "name": session_name,
            "order": order,
            "created_at": created_at,
            "backend": backend,
            "session_naming_completed": false,
            "answered_questions": [],
            "submitted_answers": {},
            "fixed_findings": [],
            "pending_permission_denials": [],
            "pending_codex_permission_requests": [],
            "pending_codex_command_approval_requests": [],
            "pending_codex_user_input_requests": [],
            "pending_codex_mcp_elicitation_requests": [],
            "pending_codex_dynamic_tool_call_requests": [],
            "is_reviewing": false,
            "waiting_for_input": false,
            "approved_plan_message_ids": [],
            "table_checked_rows": {},
            "queued_messages": [],
            "terminal_command_args": terminal_command_args.unwrap_or_default(),
            "runs": [],
            "messages": [],
            "version": 1,
        });
        let object = object_mut(&mut metadata)?;
        insert_optional_string(object, "primary_surface", primary_surface);
        insert_optional_string(object, "terminal_command", terminal_command);
        insert_optional_string(object, "terminal_label", terminal_label);
        if let Some(native_id) = native_session_id {
            let field = resume_id_field(&backend);
            object.insert(field.to_string(), Value::String(native_id.to_string()));
        }
        self.persistence.save_session_metadata(&id, &metadata)?;
        Ok(session_from_metadata(metadata))
    }

    pub fn summaries(
        &self,
        worktree_id: &str,
        include_archived: bool,
    ) -> Result<Value, BackendError> {
        let index = self
            .persistence
            .load_session_index(worktree_id)?
            .unwrap_or_else(|| empty_index(worktree_id));
        let sessions = index
            .get("sessions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let summaries = sessions
            .into_iter()
            .filter_map(|entry| {
                let id = entry.get("id")?.as_str()?;
                let metadata = self
                    .persistence
                    .load_session_metadata(id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| entry.clone());
                if !include_archived
                    && metadata
                        .get("archived_at")
                        .is_some_and(|value| !value.is_null())
                {
                    return None;
                }
                Some(summary(&metadata, &entry))
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "worktreeId": worktree_id,
            "activeSessionId": index.get("active_session_id"),
            "sessions": summaries,
        }))
    }

    pub fn list(&self, worktree_id: &str, include_archived: bool) -> Result<Value, BackendError> {
        let index = self
            .persistence
            .load_session_index(worktree_id)?
            .unwrap_or_else(|| empty_index(worktree_id));
        let sessions = index
            .get("sessions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| entry.get("id").and_then(Value::as_str))
            .filter_map(|id| self.persistence.load_session_metadata(id).ok().flatten())
            .filter(|metadata| {
                include_archived || metadata.get("archived_at").is_none_or(Value::is_null)
            })
            .map(session_from_metadata)
            .collect::<Vec<_>>();
        Ok(serde_json::json!({
            "worktree_id": worktree_id,
            "active_session_id": index.get("active_session_id"),
            "sessions": sessions,
        }))
    }

    pub fn get(&self, worktree_id: &str, session_id: &str) -> Result<Value, BackendError> {
        let mut metadata = self
            .persistence
            .load_session_metadata(session_id)?
            .ok_or_else(|| not_found(session_id))?;
        if metadata.get("worktree_id").and_then(Value::as_str) != Some(worktree_id) {
            return Err(not_found(session_id));
        }
        let messages = self.load_messages(session_id, &metadata)?;
        object_mut(&mut metadata)?.insert("messages".to_string(), Value::Array(messages));
        Ok(session_from_metadata(metadata))
    }

    pub fn status(&self, session_id: &str, actively_managed: bool) -> Result<Value, BackendError> {
        let metadata = self
            .persistence
            .load_session_metadata(session_id)?
            .ok_or_else(|| not_found(session_id))?;
        let latest = metadata
            .get("runs")
            .and_then(Value::as_array)
            .and_then(|runs| runs.last());
        let status = if actively_managed {
            "running"
        } else {
            match latest
                .and_then(|run| run.get("status"))
                .and_then(Value::as_str)
            {
                Some("running" | "resumable") => "resumable",
                Some("cancelled") => "cancelled",
                Some("crashed") => "error",
                _ => "idle",
            }
        };
        Ok(serde_json::json!({
            "sessionId": session_id,
            "worktreeId": metadata.get("worktree_id"),
            "status": status,
            "activelyManaged": actively_managed,
            "backend": metadata.get("backend"),
            "selectedModel": metadata.get("selected_model"),
            "selectedProvider": metadata.get("selected_provider"),
            "selectedExecutionMode": metadata.get("selected_execution_mode"),
            "waitingForInput": metadata.get("waiting_for_input").and_then(Value::as_bool).unwrap_or(false),
            "waitingForInputType": metadata.get("waiting_for_input_type"),
            "latestRun": latest,
        }))
    }

    pub fn rename(
        &self,
        worktree_id: &str,
        session_id: &str,
        new_name: &str,
    ) -> Result<(), BackendError> {
        let name = new_name.trim();
        if name.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Session name cannot be empty",
            ));
        }
        self.update_index_entry(worktree_id, session_id, |entry| {
            object_mut(entry)?.insert("name".to_string(), Value::String(name.to_string()));
            Ok(())
        })?;
        self.update_metadata(session_id, |metadata| {
            object_mut(metadata)?.insert("name".to_string(), Value::String(name.to_string()));
            Ok(())
        })
    }

    pub fn close(
        &self,
        worktree_id: &str,
        session_id: &str,
    ) -> Result<Option<String>, BackendError> {
        self.persistence
            .update_session_index(worktree_id, empty_index(worktree_id), |index| {
                let object = object_mut(index)?;
                let sessions = object
                    .get_mut("sessions")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| invalid_data("sessions"))?;
                let position = sessions
                    .iter()
                    .position(|entry| entry.get("id").and_then(Value::as_str) == Some(session_id))
                    .ok_or_else(|| not_found(session_id))?;
                sessions.remove(position);
                let new_active = sessions
                    .get(position.saturating_sub(1))
                    .or_else(|| sessions.first())
                    .and_then(|entry| entry.get("id"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                object.insert(
                    "active_session_id".to_string(),
                    new_active.clone().map(Value::String).unwrap_or(Value::Null),
                );
                Ok(new_active)
            })
    }

    pub fn reorder(&self, worktree_id: &str, ids: &[String]) -> Result<(), BackendError> {
        self.persistence
            .update_session_index(worktree_id, empty_index(worktree_id), |index| {
                let sessions = object_mut(index)?
                    .get_mut("sessions")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| invalid_data("sessions"))?;
                for (order, id) in ids.iter().enumerate() {
                    if let Some(entry) = sessions
                        .iter_mut()
                        .find(|entry| entry.get("id").and_then(Value::as_str) == Some(id.as_str()))
                    {
                        object_mut(entry)?.insert("order".to_string(), Value::from(order));
                    }
                }
                sessions.sort_by_key(|entry| {
                    entry
                        .get("order")
                        .and_then(Value::as_u64)
                        .unwrap_or(u64::MAX)
                });
                Ok(())
            })
    }

    pub fn set_active(&self, worktree_id: &str, session_id: &str) -> Result<(), BackendError> {
        self.persistence
            .update_session_index(worktree_id, empty_index(worktree_id), |index| {
                let object = object_mut(index)?;
                let exists = object
                    .get("sessions")
                    .and_then(Value::as_array)
                    .is_some_and(|sessions| {
                        sessions.iter().any(|entry| {
                            entry.get("id").and_then(Value::as_str) == Some(session_id)
                        })
                    });
                if !exists {
                    return Err(not_found(session_id));
                }
                object.insert(
                    "active_session_id".to_string(),
                    Value::String(session_id.to_string()),
                );
                Ok(())
            })?;
        self.update_metadata(session_id, |metadata| {
            object_mut(metadata)?.insert("last_opened_at".to_string(), Value::from(now()));
            Ok(())
        })
    }

    pub fn archive(
        &self,
        worktree_id: &str,
        session_id: &str,
        archived: bool,
    ) -> Result<Value, BackendError> {
        let archived_at = archived.then(now);
        self.update_index_entry(worktree_id, session_id, |entry| {
            let object = object_mut(entry)?;
            match archived_at {
                Some(timestamp) => {
                    object.insert("archived_at".to_string(), Value::from(timestamp));
                }
                None => {
                    object.remove("archived_at");
                }
            }
            Ok(())
        })?;
        self.update_metadata(session_id, |metadata| {
            let object = object_mut(metadata)?;
            match archived_at {
                Some(timestamp) => {
                    object.insert("archived_at".to_string(), Value::from(timestamp));
                }
                None => {
                    object.remove("archived_at");
                }
            }
            Ok(())
        })?;
        let metadata = self
            .persistence
            .load_session_metadata(session_id)?
            .ok_or_else(|| not_found(session_id))?;
        Ok(session_from_metadata(metadata))
    }

    fn update_index_entry(
        &self,
        worktree_id: &str,
        session_id: &str,
        update: impl FnOnce(&mut Value) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        self.persistence
            .update_session_index(worktree_id, empty_index(worktree_id), |index| {
                let entry = object_mut(index)?
                    .get_mut("sessions")
                    .and_then(Value::as_array_mut)
                    .and_then(|sessions| {
                        sessions.iter_mut().find(|entry| {
                            entry.get("id").and_then(Value::as_str) == Some(session_id)
                        })
                    })
                    .ok_or_else(|| not_found(session_id))?;
                update(entry)
            })
    }

    fn update_metadata(
        &self,
        session_id: &str,
        update: impl FnOnce(&mut Value) -> Result<(), BackendError>,
    ) -> Result<(), BackendError> {
        let existing = self
            .persistence
            .load_session_metadata(session_id)?
            .ok_or_else(|| not_found(session_id))?;
        self.persistence
            .update_session_metadata(session_id, existing, update)
    }

    fn load_messages(
        &self,
        session_id: &str,
        metadata: &Value,
    ) -> Result<Vec<Value>, BackendError> {
        let mut messages = Vec::new();
        for run in metadata
            .get("runs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(run_id) = run.get("run_id").and_then(Value::as_str) else {
                continue;
            };
            let status = run
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("completed");
            let cancelled = status == "cancelled";
            if cancelled && run.get("assistant_message_id").is_none() {
                continue;
            }
            messages.push(serde_json::json!({
                "id": run.get("user_message_id"),
                "session_id": session_id,
                "role": "user",
                "content": run.get("user_message").and_then(Value::as_str).unwrap_or_default(),
                "timestamp": run.get("started_at").and_then(Value::as_u64).unwrap_or_default(),
                "tool_calls": [],
                "model": run.get("model"),
                "execution_mode": run.get("execution_mode"),
                "thinking_level": run.get("thinking_level"),
                "effort_level": run.get("effort_level"),
                "cancelled": false,
                "plan_approved": false,
                "recovered": false,
            }));
            if let Some(assistant_id) = run.get("assistant_message_id").and_then(Value::as_str) {
                let path = self.persistence.run_log_path(session_id, run_id)?;
                let content = std::fs::read_to_string(path)
                    .ok()
                    .and_then(|raw| extract_assistant_text(&raw))
                    .unwrap_or_default();
                messages.push(serde_json::json!({
                    "id": assistant_id,
                    "session_id": session_id,
                    "role": "assistant",
                    "content": content,
                    "timestamp": run.get("ended_at").and_then(Value::as_u64).unwrap_or_default(),
                    "tool_calls": [],
                    "content_blocks": [],
                    "cancelled": cancelled,
                    "plan_approved": false,
                    "recovered": run.get("recovered").and_then(Value::as_bool).unwrap_or(false),
                }));
            }
        }
        Ok(messages)
    }
}

fn extract_assistant_text(raw: &str) -> Option<String> {
    let mut output = String::new();
    for line in raw.lines() {
        let value: Value = serde_json::from_str(line).ok()?;
        if value.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        for block in value
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    output.push_str(text);
                }
            }
        }
    }
    Some(output)
}

fn empty_index(worktree_id: &str) -> Value {
    serde_json::json!({
        "worktree_id": worktree_id,
        "active_session_id": Value::Null,
        "sessions": [],
        "version": 1,
        "branch_naming_completed": false,
    })
}

fn session_from_metadata(mut metadata: Value) -> Value {
    if let Some(object) = metadata.as_object_mut() {
        let total_runs = object
            .get("runs")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let updated_at = object
            .get("created_at")
            .cloned()
            .unwrap_or(Value::from(now()));
        object
            .entry("messages")
            .or_insert_with(|| Value::Array(Vec::new()));
        object
            .entry("total_runs")
            .or_insert_with(|| Value::from(total_runs));
        object
            .entry("loaded_run_start_index")
            .or_insert_with(|| Value::from(0));
        object.entry("updated_at").or_insert(updated_at);
    }
    metadata
}

fn summary(metadata: &Value, entry: &Value) -> Value {
    let runs = metadata.get("runs").and_then(Value::as_array);
    let latest = runs.and_then(|runs| runs.last());
    serde_json::json!({
        "id": metadata.get("id").or_else(|| entry.get("id")),
        "name": metadata.get("name").or_else(|| entry.get("name")),
        "order": metadata.get("order").or_else(|| entry.get("order")),
        "backend": metadata.get("backend"),
        "selectedModel": metadata.get("selected_model"),
        "selectedProvider": metadata.get("selected_provider"),
        "selectedExecutionMode": metadata.get("selected_execution_mode"),
        "createdAt": metadata.get("created_at"),
        "updatedAt": metadata.get("updated_at").or_else(|| metadata.get("created_at")),
        "lastMessageAt": metadata.get("last_message_at"),
        "messageCount": entry.get("message_count").and_then(Value::as_u64).unwrap_or(0),
        "archivedAt": metadata.get("archived_at").or_else(|| entry.get("archived_at")),
        "lastRunStatus": latest.and_then(|run| run.get("status")),
        "lastRunStartedAt": latest.and_then(|run| run.get("started_at")),
        "waitingForInput": metadata.get("waiting_for_input").and_then(Value::as_bool).unwrap_or(false),
        "waitingForInputType": metadata.get("waiting_for_input_type"),
    })
}

fn resume_id_field(backend: &str) -> &'static str {
    match backend {
        "codex" => "codex_thread_id",
        "opencode" => "opencode_session_id",
        "cursor" => "cursor_chat_id",
        "pi" => "pi_session_id",
        "commandcode" => "commandcode_session_id",
        "grok" => "grok_session_id",
        _ => "claude_session_id",
    }
}

fn insert_optional_string(object: &mut Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        object.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, BackendError> {
    value
        .as_object_mut()
        .ok_or_else(|| invalid_data("JSON object"))
}

fn invalid_data(field: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::Internal,
        format!("Invalid persisted session field '{field}'"),
    )
}

fn not_found(session_id: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("Session not found: {session_id}"),
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResolvedAppPaths;

    fn service(temp: &tempfile::TempDir) -> SessionService {
        SessionService::new(Arc::new(PersistenceService::new(Arc::new(
            ResolvedAppPaths::new(
                temp.path().join("data"),
                temp.path().join("config"),
                temp.path().join("cache"),
                temp.path().join("resources"),
            ),
        ))))
    }

    #[test]
    fn session_lifecycle_preserves_index_and_metadata_contracts() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        let created = service
            .create("w1", None, Some("codex"), None, None, None, None, None)
            .unwrap();
        let id = created["id"].as_str().unwrap();
        assert_eq!(created["name"], "Session 1");
        assert_eq!(created["backend"], "codex");
        assert_eq!(
            service.summaries("w1", false).unwrap()["sessions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        service.rename("w1", id, "Feature").unwrap();
        assert_eq!(service.get("w1", id).unwrap()["name"], "Feature");
        service.archive("w1", id, true).unwrap();
        assert!(service.summaries("w1", false).unwrap()["sessions"]
            .as_array()
            .unwrap()
            .is_empty());
        service.archive("w1", id, false).unwrap();
        assert_eq!(service.close("w1", id).unwrap(), None);
    }
}
