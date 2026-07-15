use crate::{AppPaths, BackendError, BackendErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const PREFERENCES_FILE: &str = "preferences.json";
const UI_STATE_FILE: &str = "ui-state.json";
const PROJECTS_FILE: &str = "projects.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProjectsSnapshot {
    #[serde(default)]
    pub projects: Vec<Value>,
    #[serde(default)]
    pub worktrees: Vec<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

pub struct PersistenceService {
    paths: Arc<dyn AppPaths>,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl PersistenceService {
    pub fn new(paths: Arc<dyn AppPaths>) -> Self {
        Self {
            paths,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn preferences_path(&self) -> Result<PathBuf, BackendError> {
        Ok(self.data_dir()?.join(PREFERENCES_FILE))
    }

    pub fn ui_state_path(&self) -> Result<PathBuf, BackendError> {
        Ok(self.data_dir()?.join(UI_STATE_FILE))
    }

    pub fn projects_path(&self) -> Result<PathBuf, BackendError> {
        Ok(self.data_dir()?.join(PROJECTS_FILE))
    }

    pub fn sessions_dir(&self) -> Result<PathBuf, BackendError> {
        self.ensure_dir(self.data_dir()?.join("sessions"))
    }

    pub fn git_contexts_dir(&self) -> Result<PathBuf, BackendError> {
        self.ensure_dir(self.data_dir()?.join("git-context"))
    }

    pub fn update_context_references<T>(
        &self,
        update: impl FnOnce(&mut crate::ContextReferences) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let path = self.git_contexts_dir()?.join("references.json");
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let value = load_json_or_unlocked(&path, serde_json::json!({}))?;
        let mut references: crate::ContextReferences = serde_json::from_value(value)?;
        let result = update(&mut references)?;
        save_json_unlocked(&path, &serde_json::to_value(&references)?)?;
        Ok(result)
    }

    pub fn session_index_path(&self, worktree_id: &str) -> Result<PathBuf, BackendError> {
        Ok(self
            .ensure_dir(self.sessions_dir()?.join("index"))?
            .join(format!("{}.json", sanitize_identifier(worktree_id)?)))
    }

    pub fn base_session_index_path(&self, project_id: &str) -> Result<PathBuf, BackendError> {
        Ok(self
            .ensure_dir(self.sessions_dir()?.join("index"))?
            .join(format!("base-{}.json", sanitize_identifier(project_id)?)))
    }

    pub fn session_metadata_path(&self, session_id: &str) -> Result<PathBuf, BackendError> {
        Ok(self
            .ensure_dir(
                self.ensure_dir(self.sessions_dir()?.join("data"))?
                    .join(sanitize_identifier(session_id)?),
            )?
            .join("metadata.json"))
    }

    pub fn run_log_path(&self, session_id: &str, run_id: &str) -> Result<PathBuf, BackendError> {
        Ok(self
            .session_metadata_path(session_id)?
            .parent()
            .ok_or_else(|| {
                BackendError::new(BackendErrorCode::Internal, "Invalid session metadata path")
            })?
            .join(format!("{}.jsonl", sanitize_identifier(run_id)?)))
    }

    pub fn load_preferences(&self) -> Result<Value, BackendError> {
        self.load_json_or(self.preferences_path()?, Value::Object(Map::new()))
    }

    pub fn save_preferences(&self, preferences: &Value) -> Result<(), BackendError> {
        self.save_json(self.preferences_path()?, preferences)
    }

    pub fn patch_preferences(&self, patch: &Value) -> Result<Value, BackendError> {
        let path = self.preferences_path()?;
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let mut preferences = load_json_or_unlocked(&path, Value::Object(Map::new()))?;
        merge_json(&mut preferences, patch.clone());
        save_json_unlocked(&path, &preferences)?;
        Ok(preferences)
    }

    pub fn load_ui_state(&self) -> Result<Value, BackendError> {
        self.load_json_or(self.ui_state_path()?, Value::Object(Map::new()))
    }

    pub fn save_ui_state(&self, ui_state: &Value) -> Result<(), BackendError> {
        self.save_json(self.ui_state_path()?, ui_state)
    }

    pub fn load_projects(&self) -> Result<ProjectsSnapshot, BackendError> {
        let value = self.load_json_or(
            self.projects_path()?,
            serde_json::json!({"projects": [], "worktrees": []}),
        )?;
        serde_json::from_value(value).map_err(BackendError::from)
    }

    pub fn save_projects(&self, snapshot: &ProjectsSnapshot) -> Result<(), BackendError> {
        self.save_json(self.projects_path()?, &serde_json::to_value(snapshot)?)
    }

    pub fn update_projects<T>(
        &self,
        update: impl FnOnce(&mut ProjectsSnapshot) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let path = self.projects_path()?;
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let value =
            load_json_or_unlocked(&path, serde_json::json!({"projects": [], "worktrees": []}))?;
        let mut snapshot: ProjectsSnapshot = serde_json::from_value(value)?;
        let result = update(&mut snapshot)?;
        save_json_unlocked(&path, &serde_json::to_value(&snapshot)?)?;
        Ok(result)
    }

    pub fn load_session_index(&self, worktree_id: &str) -> Result<Option<Value>, BackendError> {
        self.load_optional_json(self.session_index_path(worktree_id)?)
    }

    pub fn save_session_index(&self, worktree_id: &str, index: &Value) -> Result<(), BackendError> {
        self.save_json(self.session_index_path(worktree_id)?, index)
    }

    pub fn update_session_index<T>(
        &self,
        worktree_id: &str,
        default: Value,
        update: impl FnOnce(&mut Value) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        self.update_json(self.session_index_path(worktree_id)?, default, update)
    }

    pub fn preserve_base_session_index(
        &self,
        worktree_id: &str,
        project_id: &str,
    ) -> Result<(), BackendError> {
        let source = self.session_index_path(worktree_id)?;
        if !source.exists() {
            return Ok(());
        }
        let destination = self.base_session_index_path(project_id)?;
        fs::rename(source, destination)?;
        Ok(())
    }

    pub fn restore_base_session_index(
        &self,
        project_id: &str,
        worktree_id: &str,
    ) -> Result<Option<Value>, BackendError> {
        let preserved = self.base_session_index_path(project_id)?;
        let Some(mut index) = self.load_optional_json(preserved.clone())? else {
            return Ok(None);
        };
        index
            .as_object_mut()
            .ok_or_else(|| BackendError::new(BackendErrorCode::Io, "Invalid session index"))?
            .insert(
                "worktree_id".to_string(),
                Value::String(worktree_id.to_string()),
            );
        self.save_session_index(worktree_id, &index)?;
        fs::remove_file(preserved)?;
        Ok(Some(index))
    }

    pub fn delete_session_index(&self, worktree_id: &str) -> Result<(), BackendError> {
        remove_file_if_present(&self.session_index_path(worktree_id)?)
    }

    pub fn delete_session_data(&self, session_id: &str) -> Result<(), BackendError> {
        let metadata = self.session_metadata_path(session_id)?;
        let directory = metadata.parent().ok_or_else(|| {
            BackendError::new(BackendErrorCode::Internal, "Invalid session data path")
        })?;
        match fs::remove_dir_all(directory) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn delete_combined_contexts(&self, session_id: &str) -> Result<(), BackendError> {
        let directory = self.ensure_dir(self.data_dir()?.join("combined-contexts"))?;
        for suffix in ["combined.md", "codex-combined.md"] {
            remove_file_if_present(&directory.join(format!("{session_id}-{suffix}")))?;
        }
        Ok(())
    }

    pub fn load_session_metadata(&self, session_id: &str) -> Result<Option<Value>, BackendError> {
        self.load_optional_json(self.session_metadata_path(session_id)?)
    }

    pub fn save_session_metadata(
        &self,
        session_id: &str,
        metadata: &Value,
    ) -> Result<(), BackendError> {
        self.save_json(self.session_metadata_path(session_id)?, metadata)
    }

    pub fn update_session_metadata<T>(
        &self,
        session_id: &str,
        default: Value,
        update: impl FnOnce(&mut Value) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        self.update_json(self.session_metadata_path(session_id)?, default, update)
    }

    fn data_dir(&self) -> Result<PathBuf, BackendError> {
        self.ensure_dir(self.paths.data_dir()?)
    }

    fn ensure_dir(&self, path: PathBuf) -> Result<PathBuf, BackendError> {
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn lock_for(&self, path: &Path) -> Arc<Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    fn load_optional_json(&self, path: PathBuf) -> Result<Option<Value>, BackendError> {
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json_unlocked(&path)?))
    }

    fn load_json_or(&self, path: PathBuf, default: Value) -> Result<Value, BackendError> {
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        load_json_or_unlocked(&path, default)
    }

    fn save_json(&self, path: PathBuf, value: &Value) -> Result<(), BackendError> {
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        save_json_unlocked(&path, value)
    }

    fn update_json<T>(
        &self,
        path: PathBuf,
        default: Value,
        update: impl FnOnce(&mut Value) -> Result<T, BackendError>,
    ) -> Result<T, BackendError> {
        let lock = self.lock_for(&path);
        let _guard = lock.lock().unwrap_or_else(|error| error.into_inner());
        let mut value = load_json_or_unlocked(&path, default)?;
        let result = update(&mut value)?;
        save_json_unlocked(&path, &value)?;
        Ok(result)
    }
}

fn sanitize_identifier(identifier: &str) -> Result<String, BackendError> {
    if identifier.is_empty()
        || identifier == "."
        || identifier == ".."
        || identifier
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_'))
    {
        return Err(BackendError::new(
            BackendErrorCode::InvalidArgument,
            format!("Invalid persisted identifier '{identifier}'"),
        ));
    }
    Ok(identifier.to_string())
}

fn load_json_or_unlocked(path: &Path, default: Value) -> Result<Value, BackendError> {
    if !path.exists() {
        return Ok(default);
    }
    read_json_unlocked(path)
}

fn read_json_unlocked(path: &Path) -> Result<Value, BackendError> {
    let contents = fs::read_to_string(path)?;
    serde_json::from_str(&contents).map_err(|error| {
        BackendError::new(
            BackendErrorCode::Internal,
            format!("Failed to parse {}: {error}", path.display()),
        )
    })
}

fn save_json_unlocked(path: &Path, value: &Value) -> Result<(), BackendError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

fn merge_json(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json(target.entry(key).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch,
    }
}

fn remove_file_if_present(path: &Path) -> Result<(), BackendError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResolvedAppPaths;

    fn store(temp: &tempfile::TempDir) -> PersistenceService {
        PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().to_path_buf(),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        )))
    }

    #[test]
    fn preference_patch_preserves_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        store
            .save_preferences(&serde_json::json!({
                "theme": "dark",
                "future_field": {"keep": true},
            }))
            .unwrap();
        let patched = store
            .patch_preferences(&serde_json::json!({"theme": "light"}))
            .unwrap();
        assert_eq!(patched["theme"], "light");
        assert_eq!(patched["future_field"]["keep"], true);
    }

    #[test]
    fn session_paths_reject_traversal() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        assert_eq!(
            store.session_metadata_path("../secret").unwrap_err().code,
            BackendErrorCode::InvalidArgument
        );
    }

    #[test]
    fn atomic_save_leaves_no_temporary_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(&temp);
        store
            .save_ui_state(&serde_json::json!({"active_project_id": "p1"}))
            .unwrap();
        assert!(store.ui_state_path().unwrap().exists());
        assert!(!store
            .ui_state_path()
            .unwrap()
            .with_extension("tmp")
            .exists());
    }
}
