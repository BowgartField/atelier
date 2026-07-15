use crate::{
    read_jean_config, ActiveWorktreeInfo, BackendError, BackendErrorCode, ContextService,
    EventSink, GitService, PersistenceService, ProjectsSnapshot, ScriptService, WorktreeContexts,
};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone)]
pub struct ProjectService {
    persistence: Arc<PersistenceService>,
    git: GitService,
    scripts: ScriptService,
    contexts: ContextService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseSessionCloseMode {
    Preserve,
    Clean,
    Archive,
}

#[derive(Debug, Clone)]
pub struct ExistingBranchWorktreeInput {
    pub project_id: String,
    pub branch_name: String,
    pub contexts: WorktreeContexts,
    pub auto_open_in_jean: bool,
}

#[derive(Debug, Clone)]
pub struct ExistingBranchCreationTask {
    id: String,
    project_id: String,
    project_name: String,
    project_path: String,
    name: String,
    path: String,
    branch: String,
    created_at: u64,
    contexts: WorktreeContexts,
    auto_open_in_jean: bool,
}

impl ProjectService {
    pub fn new(persistence: Arc<PersistenceService>) -> Self {
        let git = GitService::default();
        Self {
            contexts: ContextService::new(persistence.clone(), git),
            persistence,
            git,
            scripts: ScriptService::default(),
        }
    }

    pub fn with_git(persistence: Arc<PersistenceService>, git: GitService) -> Self {
        Self {
            contexts: ContextService::new(persistence.clone(), git),
            persistence,
            git,
            scripts: ScriptService::default(),
        }
    }

    pub fn with_services(
        persistence: Arc<PersistenceService>,
        git: GitService,
        scripts: ScriptService,
        contexts: ContextService,
    ) -> Self {
        Self {
            persistence,
            git,
            scripts,
            contexts,
        }
    }

    pub fn list(&self) -> Result<Vec<Value>, BackendError> {
        Ok(self.persistence.load_projects()?.projects)
    }

    pub fn list_worktrees(&self, project_id: &str) -> Result<Vec<Value>, BackendError> {
        Ok(self
            .persistence
            .load_projects()?
            .worktrees
            .into_iter()
            .filter(|worktree| string(worktree, "project_id") == Some(project_id))
            .filter(|worktree| worktree.get("archived_at").is_none_or(Value::is_null))
            .collect())
    }

    pub fn get_worktree(&self, worktree_id: &str) -> Result<Value, BackendError> {
        self.persistence
            .load_projects()?
            .worktrees
            .into_iter()
            .find(|worktree| string(worktree, "id") == Some(worktree_id))
            .ok_or_else(|| not_found("Worktree", worktree_id))
    }

    pub fn list_archived_worktrees(&self) -> Result<Vec<Value>, BackendError> {
        Ok(self
            .persistence
            .load_projects()?
            .worktrees
            .into_iter()
            .filter(|worktree| {
                worktree
                    .get("archived_at")
                    .is_some_and(|value| !value.is_null())
            })
            .collect())
    }

    pub fn create_base_session(&self, project_id: &str) -> Result<(Value, bool), BackendError> {
        let project = self.project(project_id)?;
        let result = self.persistence.update_projects(|snapshot| {
            if let Some(existing) = snapshot.worktrees.iter().find(|worktree| {
                string(worktree, "project_id") == Some(project_id)
                    && string(worktree, "session_type") == Some("base")
            }) {
                return Ok((existing.clone(), false));
            }
            let branch = string(&project, "default_branch").unwrap_or("main");
            let path = string(&project, "path").ok_or_else(|| invalid("project.path"))?;
            let worktree = serde_json::json!({
                "id": Uuid::new_v4().to_string(),
                "project_id": project_id,
                "name": branch,
                "path": path,
                "branch": branch,
                "base_branch": Value::Null,
                "created_at": now(),
                "session_type": "base",
                "order": 0,
                "labels": [],
            });
            snapshot.worktrees.push(worktree.clone());
            Ok((worktree, true))
        })?;
        if result.1 {
            if let Some(mut index) = self.persistence.restore_base_session_index(
                project_id,
                string(&result.0, "id").unwrap_or_default(),
            )? {
                for entry in index
                    .get_mut("sessions")
                    .and_then(Value::as_array_mut)
                    .into_iter()
                    .flatten()
                {
                    let Some(session_id) = string(entry, "id").map(ToOwned::to_owned) else {
                        continue;
                    };
                    if entry.get("archived_by_base_close").and_then(Value::as_bool) == Some(true) {
                        object_mut(entry)?.remove("archived_at");
                        object_mut(entry)?.remove("archived_by_base_close");
                    }
                    if let Some(mut metadata) =
                        self.persistence.load_session_metadata(&session_id)?
                    {
                        if metadata
                            .get("archived_by_base_close")
                            .and_then(Value::as_bool)
                            == Some(true)
                        {
                            object_mut(&mut metadata)?.remove("archived_at");
                            object_mut(&mut metadata)?.remove("archived_by_base_close");
                            self.persistence
                                .save_session_metadata(&session_id, &metadata)?;
                        }
                    }
                }
                self.persistence
                    .save_session_index(string(&result.0, "id").unwrap_or_default(), &index)?;
            }
        }
        Ok(result)
    }

    pub fn close_base_session(
        &self,
        worktree_id: &str,
        mode: BaseSessionCloseMode,
        events: &dyn EventSink,
    ) -> Result<Value, BackendError> {
        let worktree = self.get_worktree(worktree_id)?;
        if string(&worktree, "session_type") != Some("base") {
            return Err(invalid("Not a base session. Use delete_worktree instead."));
        }
        let project_id = string(&worktree, "project_id")
            .ok_or_else(|| invalid("project_id"))?
            .to_string();
        match mode {
            BaseSessionCloseMode::Preserve => self
                .persistence
                .preserve_base_session_index(worktree_id, &project_id)?,
            BaseSessionCloseMode::Archive => {
                self.archive_base_sessions(worktree_id)?;
                self.persistence
                    .preserve_base_session_index(worktree_id, &project_id)?;
            }
            BaseSessionCloseMode::Clean => {
                if let Some(index) = self.persistence.load_session_index(worktree_id)? {
                    for session_id in index
                        .get("sessions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| string(entry, "id"))
                    {
                        self.persistence.delete_session_data(session_id)?;
                    }
                }
                self.persistence.delete_session_index(worktree_id)?;
            }
        }
        self.persistence.update_projects(|snapshot| {
            snapshot
                .worktrees
                .retain(|worktree| string(worktree, "id") != Some(worktree_id));
            Ok(())
        })?;
        events.emit_json(
            "worktree:deleted",
            serde_json::json!({"id":worktree_id,"project_id":project_id}),
        )?;
        Ok(worktree)
    }

    fn archive_base_sessions(&self, worktree_id: &str) -> Result<(), BackendError> {
        let Some(mut index) = self.persistence.load_session_index(worktree_id)? else {
            return Ok(());
        };
        let timestamp = now();
        for entry in index
            .get_mut("sessions")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            if entry
                .get("archived_at")
                .is_some_and(|value| !value.is_null())
            {
                continue;
            }
            let session_id = string(entry, "id").map(ToOwned::to_owned);
            object_mut(entry)?.insert("archived_at".to_string(), Value::from(timestamp));
            object_mut(entry)?.insert("archived_by_base_close".to_string(), Value::Bool(true));
            if let Some(session_id) = session_id {
                if let Some(mut metadata) = self.persistence.load_session_metadata(&session_id)? {
                    object_mut(&mut metadata)?
                        .insert("archived_at".to_string(), Value::from(timestamp));
                    object_mut(&mut metadata)?
                        .insert("archived_by_base_close".to_string(), Value::Bool(true));
                    self.persistence
                        .save_session_metadata(&session_id, &metadata)?;
                }
            }
        }
        self.persistence.save_session_index(worktree_id, &index)
    }

    pub fn archive_worktree(
        &self,
        worktree_id: &str,
        events: &dyn EventSink,
    ) -> Result<(), BackendError> {
        let project_id = self.persistence.update_projects(|snapshot| {
            let worktree = snapshot
                .worktrees
                .iter_mut()
                .find(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            if string(worktree, "session_type") == Some("base") {
                return Err(invalid(
                    "Base sessions cannot be archived. Use close_base_session instead.",
                ));
            }
            if worktree
                .get("archived_at")
                .is_some_and(|value| !value.is_null())
            {
                return Err(invalid("Worktree is already archived"));
            }
            let project_id = string(worktree, "project_id")
                .ok_or_else(|| invalid("project_id"))?
                .to_string();
            object_mut(worktree)?.insert("archived_at".to_string(), Value::from(now()));
            Ok(project_id)
        })?;
        events.emit_json(
            "worktree:archived",
            serde_json::json!({"id":worktree_id,"project_id":project_id}),
        )
    }

    pub fn unarchive_worktree(
        &self,
        worktree_id: &str,
        events: &dyn EventSink,
    ) -> Result<Value, BackendError> {
        let restored = self.persistence.update_projects(|snapshot| {
            let worktree = snapshot
                .worktrees
                .iter_mut()
                .find(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            if worktree.get("archived_at").is_none_or(Value::is_null) {
                return Err(invalid("Worktree is not archived"));
            }
            if string(worktree, "session_type") != Some("base") {
                let path = string(worktree, "path").ok_or_else(|| invalid("worktree.path"))?;
                if !Path::new(path).exists() {
                    return Err(invalid(format!(
                        "Git worktree directory no longer exists: {path}. The worktree may need to be permanently deleted."
                    )));
                }
            }
            object_mut(worktree)?.remove("archived_at");
            Ok(worktree.clone())
        })?;
        events.emit_json(
            "worktree:unarchived",
            serde_json::json!({"worktree":restored}),
        )?;
        Ok(restored)
    }

    pub fn add(&self, path: String, parent_id: Option<String>) -> Result<Value, BackendError> {
        let repo_path = Path::new(&path);
        if !self.git.is_repository(&path)? {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("The selected folder is not a git repository: {path}"),
            ));
        }
        let name = repo_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                BackendError::new(BackendErrorCode::InvalidArgument, "Invalid repository path")
            })?
            .to_string();
        let default_branch = self
            .git
            .current_branch(&path)
            .unwrap_or_else(|_| "main".to_string());

        self.persistence.update_projects(move |snapshot| {
            if snapshot
                .projects
                .iter()
                .any(|project| string(project, "path") == Some(path.as_str()))
            {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    format!("Project already exists: {path}"),
                ));
            }
            if let Some(parent) = parent_id.as_deref() {
                let valid_parent = snapshot.projects.iter().any(|project| {
                    string(project, "id") == Some(parent)
                        && project
                            .get("is_folder")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                });
                if !valid_parent {
                    return Err(not_found("Project folder", parent));
                }
            }
            let order = next_order(snapshot, parent_id.as_deref());
            let project = serde_json::json!({
                "id": Uuid::new_v4().to_string(),
                "name": name,
                "path": path,
                "default_branch": default_branch,
                "added_at": now(),
                "order": order,
                "parent_id": parent_id,
                "is_folder": false,
                "known_mcp_servers": [],
                "remote_clones": [],
                "linked_project_ids": [],
            });
            snapshot.projects.push(project.clone());
            Ok(project)
        })
    }

    pub fn init(&self, path: String, parent_id: Option<String>) -> Result<Value, BackendError> {
        self.git.init_repository(&path)?;
        self.add(path, parent_id)
    }

    pub fn clone_repository(
        &self,
        url: &str,
        path: String,
        parent_id: Option<String>,
    ) -> Result<Value, BackendError> {
        if url.trim().is_empty() || path.trim().is_empty() {
            return Err(invalid("url or path"));
        }
        self.git.clone_repository(url, &path)?;
        self.add(path, parent_id)
    }

    pub fn branches(&self, project_id: &str) -> Result<Vec<String>, BackendError> {
        let project = self.project(project_id)?;
        let path = Path::new(string(&project, "path").ok_or_else(|| invalid("project.path"))?);
        self.git.project_branches(path.to_string_lossy().as_ref())
    }

    pub fn create_worktree(
        &self,
        project_id: &str,
        base_branch: Option<&str>,
        custom_name: Option<&str>,
        origin: Option<&str>,
        events: &dyn EventSink,
    ) -> Result<Value, BackendError> {
        let project = self.project(project_id)?;
        let project_path = string(&project, "path").ok_or_else(|| invalid("project.path"))?;
        let project_name = string(&project, "name").ok_or_else(|| invalid("project.name"))?;
        let base = base_branch
            .or_else(|| string(&project, "default_branch"))
            .unwrap_or("main");
        let name = custom_name
            .map(sanitize_branch_name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| format!("worktree-{}", &Uuid::new_v4().simple().to_string()[..8]));
        if self
            .persistence
            .load_projects()?
            .worktrees
            .iter()
            .any(|worktree| {
                string(worktree, "project_id") == Some(project_id)
                    && string(worktree, "name") == Some(name.as_str())
            })
        {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Worktree already exists: {name}"),
            ));
        }
        let root = string(&project, "worktrees_dir")
            .map(Path::new)
            .map(ToOwned::to_owned)
            .or_else(|| dirs::home_dir().map(|home| home.join("jean")))
            .ok_or_else(|| BackendError::new(BackendErrorCode::Io, "Home directory not found"))?;
        let path = root.join(project_name).join(name.replace('/', "-"));
        if path.exists() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                format!("Worktree path already exists: {}", path.display()),
            ));
        }
        let id = Uuid::new_v4().to_string();
        let path_string = path.to_string_lossy().into_owned();
        events.emit_json(
            "worktree:creating",
            serde_json::json!({
                "id": id,
                "projectId": project_id,
                "name": name,
                "path": path_string,
                "branch": name,
                "prNumber": Value::Null,
                "issueNumber": Value::Null,
                "securityAlertNumber": Value::Null,
                "advisoryGhsaId": Value::Null,
                "origin": origin,
                "autoOpenInJean": true,
            }),
        )?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let branch_exists = self.git.branch_exists(project_path, &name);
        if branch_exists {
            self.git
                .create_worktree_from_existing_branch(project_path, &path_string, &name)?;
        } else {
            self.git
                .create_worktree(project_path, &path_string, &name, base)?;
        }
        if !path.exists() {
            let error = BackendError::new(
                BackendErrorCode::Io,
                "git worktree add did not create the requested directory",
            );
            let _ = events.emit_json(
                "worktree:error",
                serde_json::json!({"id": id, "project_id": project_id, "error": error.message}),
            );
            return Err(error);
        }
        let order = self
            .persistence
            .load_projects()?
            .worktrees
            .iter()
            .filter(|worktree| string(worktree, "project_id") == Some(project_id))
            .filter_map(|worktree| worktree.get("order").and_then(Value::as_u64))
            .max()
            .map_or(1, |value| value + 1);
        let worktree = serde_json::json!({
            "id": id,
            "project_id": project_id,
            "name": name,
            "path": path_string,
            "branch": name,
            "base_branch": base,
            "created_at": now(),
            "session_type": "worktree",
            "order": order,
            "origin": origin,
            "labels": [],
        });
        self.persistence.update_projects(|snapshot| {
            snapshot.worktrees.push(worktree.clone());
            Ok(())
        })?;
        events.emit_json(
            "worktree:created",
            serde_json::json!({"worktree": worktree, "autoOpenInJean": true}),
        )?;
        let Some(script) = read_jean_config(project_path).and_then(|config| config.scripts.setup)
        else {
            return Ok(worktree);
        };
        let setup_result = self
            .scripts
            .run_setup(&path_string, project_path, &name, &script);
        let (output, success) = match setup_result {
            Ok(output) => (output, true),
            Err(error) => (error.to_string(), false),
        };
        let updated = self.persistence.update_projects(|snapshot| {
            let stored = snapshot
                .worktrees
                .iter_mut()
                .find(|candidate| string(candidate, "id") == Some(id.as_str()))
                .ok_or_else(|| not_found("Worktree", &id))?;
            let object = object_mut(stored)?;
            object.insert("setup_output".to_string(), Value::String(output.clone()));
            object.insert("setup_script".to_string(), Value::String(script.clone()));
            object.insert("setup_success".to_string(), Value::Bool(success));
            Ok(stored.clone())
        })?;
        events.emit_json(
            "worktree:setup_complete",
            serde_json::json!({
                "id": id,
                "project_id": project_id,
                "setup_output": output,
                "setup_script": script,
                "setup_success": success,
            }),
        )?;
        Ok(updated)
    }

    pub fn prepare_existing_branch_worktree(
        &self,
        input: ExistingBranchWorktreeInput,
        events: &dyn EventSink,
    ) -> Result<(Value, ExistingBranchCreationTask), BackendError> {
        let project = self.project(&input.project_id)?;
        let project_name = string(&project, "name")
            .ok_or_else(|| invalid("project.name"))?
            .to_string();
        let project_path = string(&project, "path")
            .ok_or_else(|| invalid("project.path"))?
            .to_string();
        let root = string(&project, "worktrees_dir")
            .map(Path::new)
            .map(ToOwned::to_owned)
            .or_else(|| dirs::home_dir().map(|home| home.join("jean")))
            .ok_or_else(|| BackendError::new(BackendErrorCode::Io, "Home directory not found"))?;
        let name = input.branch_name.clone();
        let path = root
            .join(&project_name)
            .join(sanitize_folder_name(&name))
            .to_string_lossy()
            .into_owned();
        let id = Uuid::new_v4().to_string();
        let created_at = now();
        events.emit_json(
            "worktree:creating",
            serde_json::json!({
                "id":id,
                "projectId":input.project_id,
                "name":name,
                "path":path,
                "branch":input.branch_name,
                "prNumber":input.contexts.pull_request.as_ref().map(|context| context.number),
                "issueNumber":input.contexts.issue.as_ref().map(|context| context.number),
                "securityAlertNumber":input.contexts.security.as_ref().map(|context| context.number),
                "advisoryGhsaId":input.contexts.advisory.as_ref().map(|context| context.ghsa_id.clone()),
                "origin":Value::Null,
                "autoOpenInJean":input.auto_open_in_jean,
            }),
        )?;
        let pending = worktree_value(
            &id,
            &input.project_id,
            &name,
            &path,
            &input.branch_name,
            created_at,
            0,
            &input.contexts,
            None,
        );
        let task = ExistingBranchCreationTask {
            id,
            project_id: input.project_id,
            project_name,
            project_path,
            name,
            path,
            branch: input.branch_name,
            created_at,
            contexts: input.contexts,
            auto_open_in_jean: input.auto_open_in_jean,
        };
        Ok((pending, task))
    }

    pub fn complete_existing_branch_worktree(
        &self,
        task: ExistingBranchCreationTask,
        events: &dyn EventSink,
    ) -> Result<Value, BackendError> {
        let path = Path::new(&task.path);
        if path.exists() {
            let snapshot = self.persistence.load_projects()?;
            let archived = snapshot.worktrees.iter().find(|worktree| {
                string(worktree, "path") == Some(task.path.as_str())
                    && worktree
                        .get("archived_at")
                        .is_some_and(|value| !value.is_null())
            });
            let suggested_name = unique_suffix_name(
                &task.name,
                &task.project_path,
                &task.project_id,
                &snapshot,
                self.git,
            );
            events.emit_json(
                "worktree:path_exists",
                serde_json::json!({
                    "id":task.id,
                    "project_id":task.project_id,
                    "path":task.path,
                    "suggested_name":suggested_name,
                    "archived_worktree_id":archived.and_then(|worktree| string(worktree,"id")),
                    "archived_worktree_name":archived.and_then(|worktree| string(worktree,"name")),
                    "issue_context":task.contexts.issue,
                    "pr_context":task.contexts.pull_request,
                    "security_context":task.contexts.security,
                    "advisory_context":task.contexts.advisory,
                    "origin":Value::Null,
                }),
            )?;
            return self.creation_error(
                events,
                &task,
                format!("Directory already exists: {}", task.path),
            );
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Err(error) = self.git.create_worktree_from_existing_branch(
            &task.project_path,
            &task.path,
            &task.branch,
        ) {
            return self.creation_error(events, &task, error.message);
        }
        if let Err(error) = self.contexts.write_worktree_contexts(
            &task.project_path,
            &task.project_name,
            &task.id,
            &task.contexts,
        ) {
            log::warn!(
                "Failed to persist worktree contexts for {}: {error}",
                task.id
            );
        }
        let setup = read_jean_config(&task.project_path)
            .and_then(|config| config.scripts.setup)
            .map(|script| {
                let result =
                    self.scripts
                        .run_setup(&task.path, &task.project_path, &task.name, &script);
                match result {
                    Ok(output) => (output, script, true),
                    Err(error) => (error.to_string(), script, false),
                }
            });
        let worktree = self.persistence.update_projects(|snapshot| {
            if snapshot
                .worktrees
                .iter()
                .any(|worktree| string(worktree, "id") == Some(task.id.as_str()))
            {
                return Err(invalid(format!("Worktree already exists: {}", task.id)));
            }
            let order = snapshot
                .worktrees
                .iter()
                .filter(|worktree| string(worktree, "project_id") == Some(task.project_id.as_str()))
                .filter_map(|worktree| worktree.get("order").and_then(Value::as_u64))
                .max()
                .map_or(1, |order| order + 1);
            let worktree = worktree_value(
                &task.id,
                &task.project_id,
                &task.name,
                &task.path,
                &task.branch,
                task.created_at,
                order,
                &task.contexts,
                setup.as_ref(),
            );
            snapshot.worktrees.push(worktree.clone());
            Ok(worktree)
        })?;
        events.emit_json(
            "worktree:created",
            serde_json::json!({
                "worktree":worktree,
                "autoOpenInJean":task.auto_open_in_jean,
            }),
        )?;
        Ok(worktree)
    }

    pub fn run_existing_branch_worktree_task(
        &self,
        task: ExistingBranchCreationTask,
        events: &dyn EventSink,
    ) {
        let panic_task = task.clone();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.complete_existing_branch_worktree(task, events)
        }));
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => log::warn!(
                "Existing-branch worktree creation {} failed: {error}",
                panic_task.id
            ),
            Err(_) => {
                let _ = self.creation_error(
                    events,
                    &panic_task,
                    "Internal error: worktree creation failed unexpectedly".to_string(),
                );
            }
        }
    }

    fn creation_error(
        &self,
        events: &dyn EventSink,
        task: &ExistingBranchCreationTask,
        message: String,
    ) -> Result<Value, BackendError> {
        events.emit_json(
            "worktree:error",
            serde_json::json!({"id":task.id,"project_id":task.project_id,"error":message}),
        )?;
        Err(BackendError::new(BackendErrorCode::Io, message))
    }

    pub fn import_worktree(
        &self,
        project_id: &str,
        path: &str,
        events: &dyn EventSink,
    ) -> Result<Value, BackendError> {
        let worktree_path = Path::new(path);
        if !worktree_path.exists() {
            return Err(invalid(format!("Path does not exist: {path}")));
        }
        if !worktree_path.is_dir() {
            return Err(invalid(format!("Path is not a directory: {path}")));
        }
        if !worktree_path.join(".git").exists() {
            return Err(invalid(format!(
                "Path is not a git worktree or repository: {path}"
            )));
        }
        self.project(project_id)?;
        let branch = self.git.current_branch(path)?;
        let name = worktree_path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .ok_or_else(|| invalid(format!("Invalid path: {path}")))?
            .to_string();
        let worktree = self.persistence.update_projects(|snapshot| {
            if snapshot
                .worktrees
                .iter()
                .any(|worktree| string(worktree, "path") == Some(path))
            {
                return Err(invalid(format!(
                    "A worktree with this path is already tracked: {path}"
                )));
            }
            let order = snapshot
                .worktrees
                .iter()
                .filter(|worktree| string(worktree, "project_id") == Some(project_id))
                .filter_map(|worktree| worktree.get("order").and_then(Value::as_u64))
                .max()
                .map_or(1, |order| order + 1);
            let worktree = serde_json::json!({
                "id": Uuid::new_v4().to_string(),
                "project_id": project_id,
                "name": name,
                "path": path,
                "branch": branch,
                "base_branch": Value::Null,
                "created_at": now(),
                "session_type": "worktree",
                "order": order,
                "labels": [],
            });
            snapshot.worktrees.push(worktree.clone());
            Ok(worktree)
        })?;
        events.emit_json(
            "worktree:created",
            serde_json::json!({"worktree": worktree, "autoOpenInJean": true}),
        )?;
        Ok(worktree)
    }

    pub fn permanently_delete_worktree(
        &self,
        worktree_id: &str,
        events: &dyn EventSink,
    ) -> Result<(), BackendError> {
        let worktree = self.get_worktree(worktree_id)?;
        if worktree.get("archived_at").is_none_or(Value::is_null) {
            return Err(invalid(
                "Only archived worktrees can be permanently deleted. Archive it first.",
            ));
        }
        let project_id = string(&worktree, "project_id")
            .ok_or_else(|| invalid("project_id"))?
            .to_string();
        let project = self.project(&project_id)?;
        let project_path = string(&project, "path")
            .ok_or_else(|| invalid("project.path"))?
            .to_string();
        let worktree_path = string(&worktree, "path")
            .ok_or_else(|| invalid("worktree.path"))?
            .to_string();
        let branch = string(&worktree, "branch").map(ToOwned::to_owned);
        let is_base = string(&worktree, "session_type") == Some("base");
        let session_ids = self
            .persistence
            .load_session_index(worktree_id)?
            .and_then(|index| index.get("sessions").and_then(Value::as_array).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|session| string(&session, "id").map(ToOwned::to_owned))
            .collect::<Vec<_>>();

        self.persistence.update_projects(|snapshot| {
            snapshot
                .worktrees
                .retain(|candidate| string(candidate, "id") != Some(worktree_id));
            Ok(())
        })?;

        if !is_base {
            if let Err(error) = self.git.remove_worktree(&project_path, &worktree_path) {
                log::warn!("Failed to remove archived worktree {worktree_path}: {error}");
            }
            if let Some(branch) = branch {
                if let Err(error) = self.git.delete_branch(&project_path, &branch) {
                    log::warn!("Failed to delete archived worktree branch {branch}: {error}");
                }
            }
        }
        for session_id in session_ids {
            self.persistence.delete_session_data(&session_id)?;
            self.persistence.delete_combined_contexts(&session_id)?;
        }
        self.persistence.delete_session_index(worktree_id)?;
        events.emit_json(
            "worktree:permanently_deleted",
            serde_json::json!({"id":worktree_id,"project_id":project_id}),
        )
    }

    pub fn delete_worktree(
        &self,
        worktree_id: &str,
        events: &dyn EventSink,
    ) -> Result<(), BackendError> {
        let worktree = self.get_worktree(worktree_id)?;
        if string(&worktree, "session_type") == Some("base") {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Cannot delete a base session",
            ));
        }
        let project_id = string(&worktree, "project_id").ok_or_else(|| invalid("project_id"))?;
        let project = self.project(project_id)?;
        let project_path = string(&project, "path").ok_or_else(|| invalid("project.path"))?;
        let worktree_path = string(&worktree, "path").ok_or_else(|| invalid("worktree.path"))?;
        events.emit_json(
            "worktree:deleting",
            serde_json::json!({"id": worktree_id, "project_id": project_id}),
        )?;
        if let Err(error) = self.git.remove_worktree(project_path, worktree_path) {
            events.emit_json(
                "worktree:delete_error",
                serde_json::json!({"id": worktree_id, "project_id": project_id, "error": error.message}),
            )?;
            return Err(error);
        }
        if let Some(branch) = string(&worktree, "branch") {
            if let Err(error) = self.git.delete_branch(project_path, branch) {
                events.emit_json(
                    "worktree:delete_error",
                    serde_json::json!({"id": worktree_id, "project_id": project_id, "error": error.message}),
                )?;
                return Err(error);
            }
        }
        self.persistence.update_projects(|snapshot| {
            snapshot
                .worktrees
                .retain(|worktree| string(worktree, "id") != Some(worktree_id));
            Ok(())
        })?;
        events.emit_json(
            "worktree:deleted",
            serde_json::json!({"id": worktree_id, "project_id": project_id}),
        )?;
        Ok(())
    }

    pub fn worktree_changes(
        &self,
        worktree_id: &str,
        max_files: usize,
    ) -> Result<Value, BackendError> {
        let worktree = self.get_worktree(worktree_id)?;
        let path = string(&worktree, "path").ok_or_else(|| invalid("worktree.path"))?;
        let project_id = string(&worktree, "project_id").ok_or_else(|| invalid("project_id"))?;
        let project = self.project(project_id)?;
        let base = string(&worktree, "base_branch")
            .or_else(|| string(&project, "default_branch"))
            .unwrap_or("main");
        let status = self
            .git
            .branch_status(&ActiveWorktreeInfo {
                worktree_id: worktree_id.to_string(),
                worktree_path: path.to_string(),
                base_branch: base.to_string(),
                pr_number: u32_value(&worktree, "pr_number"),
                pr_url: string(&worktree, "pr_url").map(ToOwned::to_owned),
                pr_push_remote: string(&worktree, "pr_push_remote").map(ToOwned::to_owned),
                pr_push_branch: string(&worktree, "pr_push_branch").map(ToOwned::to_owned),
            })
            .ok();
        let porcelain = self
            .git
            .text(Path::new(path), &["status", "--porcelain=v1"])?;
        let mut files = parse_porcelain_files(&porcelain)
            .into_iter()
            .map(|(status, path)| serde_json::json!({"status": status, "path": path}))
            .collect::<Vec<_>>();
        let truncated = files.len() > max_files;
        files.truncate(max_files);
        Ok(serde_json::json!({
            "worktreeId": worktree_id,
            "worktreePath": path,
            "branch": worktree.get("branch"),
            "baseBranch": base,
            "status": status,
            "files": files,
            "filesTruncated": truncated,
            "porcelain": porcelain,
        }))
    }

    pub fn worktree_diff(
        &self,
        worktree_id: &str,
        diff_type: &str,
        file_path: Option<&str>,
        max_bytes: usize,
    ) -> Result<Value, BackendError> {
        let worktree = self.get_worktree(worktree_id)?;
        let path = string(&worktree, "path").ok_or_else(|| invalid("worktree.path"))?;
        let project_id = string(&worktree, "project_id").ok_or_else(|| invalid("project_id"))?;
        let project = self.project(project_id)?;
        let base = string(&worktree, "base_branch")
            .or_else(|| string(&project, "default_branch"))
            .unwrap_or("main");
        let has_head = self
            .git
            .run(Path::new(path), &["rev-parse", "--verify", "HEAD"])
            .is_ok_and(|output| output.status.success());
        let mut owned_args = match diff_type {
            "uncommitted" => vec![
                "diff".to_string(),
                if has_head {
                    "HEAD"
                } else {
                    "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
                }
                .to_string(),
                "--unified=3".to_string(),
            ],
            "branch" => vec![
                "diff".to_string(),
                "--unified=3".to_string(),
                format!("origin/{base}...HEAD"),
            ],
            _ => return Err(invalid("diffType")),
        };
        if let Some(file_path) = file_path.filter(|path| !path.trim().is_empty()) {
            owned_args.extend(["--".to_string(), file_path.to_string()]);
        }
        let args = owned_args.iter().map(String::as_str).collect::<Vec<_>>();
        let raw = self.git.text(Path::new(path), &args)?;
        let truncated = raw.len() > max_bytes;
        let patch = truncate_utf8(&raw, max_bytes);
        Ok(
            serde_json::json!({"worktreeId":worktree_id,"diffType":diff_type,"baseBranch":base,"path":file_path,"maxBytes":max_bytes,"truncated":truncated,"rawBytes":raw.len(),"patch":patch}),
        )
    }

    pub fn reorder_worktrees(&self, project_id: &str, ids: &[String]) -> Result<(), BackendError> {
        self.persistence.update_projects(|snapshot| {
            let mut next_order = 1_u64;
            for id in ids {
                if let Some(worktree) = snapshot.worktrees.iter_mut().find(|worktree| {
                    string(worktree, "id") == Some(id)
                        && string(worktree, "project_id") == Some(project_id)
                }) {
                    if string(worktree, "session_type") != Some("base") {
                        object_mut(worktree)?.insert("order".to_string(), Value::from(next_order));
                        next_order += 1;
                    }
                }
            }
            Ok(())
        })
    }

    pub fn rename_worktree(
        &self,
        worktree_id: &str,
        new_name: &str,
    ) -> Result<Value, BackendError> {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            return Err(invalid("Name cannot be empty"));
        }
        self.persistence.update_projects(|snapshot| {
            let index = snapshot
                .worktrees
                .iter()
                .position(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            let project_id = string(&snapshot.worktrees[index], "project_id")
                .ok_or_else(|| invalid("project_id"))?;
            if snapshot.worktrees.iter().any(|worktree| {
                string(worktree, "id") != Some(worktree_id)
                    && string(worktree, "project_id") == Some(project_id)
                    && string(worktree, "name") == Some(new_name)
            }) {
                return Err(invalid(format!(
                    "A worktree named '{new_name}' already exists in this project"
                )));
            }
            object_mut(&mut snapshot.worktrees[index])?
                .insert("name".to_string(), Value::String(new_name.to_string()));
            Ok(snapshot.worktrees[index].clone())
        })
    }

    pub fn update_worktree_labels(
        &self,
        worktree_id: &str,
        labels: Vec<Value>,
    ) -> Result<(), BackendError> {
        let mut seen = HashSet::new();
        let mut deduplicated = Vec::new();
        for label in labels {
            let name = string(&label, "name").ok_or_else(|| invalid("labels.name"))?;
            if label.get("color").and_then(Value::as_str).is_none() {
                return Err(invalid("labels.color"));
            }
            if seen.insert(name.to_lowercase()) {
                deduplicated.push(label);
            }
        }
        self.persistence.update_projects(|snapshot| {
            let worktree = snapshot
                .worktrees
                .iter_mut()
                .find(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            let object = object_mut(worktree)?;
            object.insert("labels".to_string(), Value::Array(deduplicated));
            object.remove("label");
            Ok(())
        })
    }

    pub fn set_worktree_last_opened(&self, worktree_id: &str) -> Result<(), BackendError> {
        self.persistence.update_projects(|snapshot| {
            let worktree = snapshot
                .worktrees
                .iter_mut()
                .find(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            object_mut(worktree)?.insert("last_opened_at".to_string(), Value::from(now()));
            Ok(())
        })
    }

    pub fn update_worktree_cached_status(
        &self,
        worktree_id: &str,
        args: &Value,
    ) -> Result<(), BackendError> {
        self.persistence.update_projects(|snapshot| {
            let worktree = snapshot
                .worktrees
                .iter_mut()
                .find(|worktree| string(worktree, "id") == Some(worktree_id))
                .ok_or_else(|| not_found("Worktree", worktree_id))?;
            let is_base = string(worktree, "session_type") == Some("base");
            let object = object_mut(worktree)?;
            if let Some(branch) = optional_string_value(args, "branch", "branch")? {
                object.insert("branch".to_string(), Value::String(branch.to_string()));
                if is_base {
                    object.insert("name".to_string(), Value::String(branch.to_string()));
                }
            }
            for (camel, snake, stored) in [
                ("prStatus", "pr_status", "cached_pr_status"),
                ("checkStatus", "check_status", "cached_check_status"),
            ] {
                if let Some(value) = optional_string_value(args, camel, snake)? {
                    object.insert(stored.to_string(), Value::String(value.to_string()));
                }
            }
            for (camel, snake, stored) in [
                ("behindCount", "behind_count", "cached_behind_count"),
                ("aheadCount", "ahead_count", "cached_ahead_count"),
                (
                    "uncommittedAdded",
                    "uncommitted_added",
                    "cached_uncommitted_added",
                ),
                (
                    "uncommittedRemoved",
                    "uncommitted_removed",
                    "cached_uncommitted_removed",
                ),
                (
                    "branchDiffAdded",
                    "branch_diff_added",
                    "cached_branch_diff_added",
                ),
                (
                    "branchDiffRemoved",
                    "branch_diff_removed",
                    "cached_branch_diff_removed",
                ),
                (
                    "baseBranchAheadCount",
                    "base_branch_ahead_count",
                    "cached_base_branch_ahead_count",
                ),
                (
                    "baseBranchBehindCount",
                    "base_branch_behind_count",
                    "cached_base_branch_behind_count",
                ),
                (
                    "worktreeAheadCount",
                    "worktree_ahead_count",
                    "cached_worktree_ahead_count",
                ),
                ("unpushedCount", "unpushed_count", "cached_unpushed_count"),
            ] {
                if let Some(value) = optional_u32_value(args, camel, snake)? {
                    object.insert(stored.to_string(), Value::from(value));
                }
            }
            object.insert("cached_status_at".to_string(), Value::from(now()));
            Ok(())
        })
    }

    pub fn remove(&self, project_id: &str) -> Result<(), BackendError> {
        let (archived_ids, removed) = self.persistence.update_projects(|snapshot| {
            if snapshot.worktrees.iter().any(|worktree| {
                string(worktree, "project_id") == Some(project_id)
                    && worktree.get("archived_at").is_none_or(Value::is_null)
            }) {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidArgument,
                    "Cannot remove project with existing worktrees. Delete worktrees first.",
                ));
            }
            let archived_ids = snapshot
                .worktrees
                .iter()
                .filter(|worktree| string(worktree, "project_id") == Some(project_id))
                .filter_map(|worktree| string(worktree, "id").map(ToOwned::to_owned))
                .collect::<Vec<_>>();
            snapshot
                .worktrees
                .retain(|worktree| string(worktree, "project_id") != Some(project_id));
            for project in &mut snapshot.projects {
                if let Some(linked) = project
                    .get_mut("linked_project_ids")
                    .and_then(Value::as_array_mut)
                {
                    linked.retain(|id| id.as_str() != Some(project_id));
                }
            }
            let original_len = snapshot.projects.len();
            snapshot
                .projects
                .retain(|project| string(project, "id") != Some(project_id));
            Ok((archived_ids, snapshot.projects.len() != original_len))
        })?;
        if !removed {
            return Err(not_found("Project", project_id));
        }
        for id in archived_ids {
            if let Ok(path) = self.persistence.session_index_path(&id) {
                remove_file_if_present(&path);
            }
        }
        Ok(())
    }

    pub fn update(&self, project_id: &str, args: &Value) -> Result<Value, BackendError> {
        self.persistence.update_projects(|snapshot| {
            let index = snapshot
                .projects
                .iter()
                .position(|project| string(project, "id") == Some(project_id))
                .ok_or_else(|| not_found("Project", project_id))?;
            let old_links = string_array(&snapshot.projects[index], "linked_project_ids");
            let project = object_mut(&mut snapshot.projects[index])?;

            update_trimmed_nonempty(project, args, "name", "name")?;
            update_string(project, args, "defaultBranch", "default_branch", false)?;
            update_array(project, args, "enabledMcpServers", "enabled_mcp_servers")?;
            update_array(project, args, "knownMcpServers", "known_mcp_servers")?;
            update_optional_trimmed(project, args, "customSystemPrompt", "custom_system_prompt")?;
            update_nullable_string(project, args, "defaultProvider", "default_provider", true)?;
            update_nullable_string(project, args, "defaultBackend", "default_backend", true)?;
            update_optional_trimmed(project, args, "worktreesDir", "worktrees_dir")?;
            update_optional_trimmed(project, args, "linearApiKey", "linear_api_key")?;
            update_optional_trimmed(project, args, "linearTeamId", "linear_team_id")?;
            update_nullable_value(project, args, "autoFixSettings", "auto_fix_settings")?;

            if let Some(value) = dual_field(args, "linkedProjectIds", "linked_project_ids") {
                let ids = value
                    .as_array()
                    .ok_or_else(|| invalid("linkedProjectIds"))?;
                let mut seen = HashSet::new();
                let clean = ids
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|id| *id != project_id && seen.insert((*id).to_string()))
                    .map(|id| Value::String(id.to_string()))
                    .collect::<Vec<_>>();
                project.insert(
                    "linked_project_ids".to_string(),
                    Value::Array(clean.clone()),
                );
                let clean_ids = clean
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<HashSet<_>>();
                for other in &mut snapshot.projects {
                    let Some(other_id) = string(other, "id").map(ToOwned::to_owned) else {
                        continue;
                    };
                    if other_id == project_id {
                        continue;
                    }
                    let should_link = clean_ids.contains(other_id.as_str());
                    let was_linked = old_links.iter().any(|id| id == &other_id);
                    if should_link || was_linked {
                        set_reciprocal_link(other, project_id, should_link)?;
                    }
                }
            }
            Ok(snapshot.projects[index].clone())
        })
    }

    pub fn reorder(&self, project_ids: &[String]) -> Result<(), BackendError> {
        self.persistence.update_projects(|snapshot| {
            for (order, id) in project_ids.iter().enumerate() {
                if let Some(project) = snapshot
                    .projects
                    .iter_mut()
                    .find(|project| string(project, "id") == Some(id))
                {
                    object_mut(project)?.insert("order".to_string(), Value::from(order));
                }
            }
            snapshot.projects.sort_by_key(|project| {
                project
                    .get("order")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX)
            });
            Ok(())
        })
    }

    fn project(&self, project_id: &str) -> Result<Value, BackendError> {
        self.persistence
            .load_projects()?
            .projects
            .into_iter()
            .find(|project| string(project, "id") == Some(project_id))
            .ok_or_else(|| not_found("Project", project_id))
    }
}

fn next_order(snapshot: &ProjectsSnapshot, parent_id: Option<&str>) -> u64 {
    snapshot
        .projects
        .iter()
        .filter(|project| string(project, "parent_id") == parent_id)
        .filter_map(|project| project.get("order").and_then(Value::as_u64))
        .max()
        .map_or(0, |order| order + 1)
}

fn set_reciprocal_link(
    project: &mut Value,
    project_id: &str,
    linked: bool,
) -> Result<(), BackendError> {
    let object = object_mut(project)?;
    let links = object
        .entry("linked_project_ids")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| invalid("linked_project_ids"))?;
    links.retain(|value| value.as_str() != Some(project_id));
    if linked {
        links.push(Value::String(project_id.to_string()));
    }
    Ok(())
}

fn update_trimmed_nonempty(
    object: &mut Map<String, Value>,
    args: &Value,
    key: &str,
    stored: &str,
) -> Result<(), BackendError> {
    if let Some(value) = args.get(key) {
        let value = value.as_str().ok_or_else(|| invalid(key))?.trim();
        if value.is_empty() {
            return Err(BackendError::new(
                BackendErrorCode::InvalidArgument,
                "Project name cannot be empty",
            ));
        }
        object.insert(stored.to_string(), Value::String(value.to_string()));
    }
    Ok(())
}

fn update_string(
    object: &mut Map<String, Value>,
    args: &Value,
    camel: &str,
    snake: &str,
    trim: bool,
) -> Result<(), BackendError> {
    if let Some(value) = dual_field(args, camel, snake) {
        let value = value.as_str().ok_or_else(|| invalid(camel))?;
        object.insert(
            snake.to_string(),
            Value::String(if trim { value.trim() } else { value }.to_string()),
        );
    }
    Ok(())
}

fn update_array(
    object: &mut Map<String, Value>,
    args: &Value,
    camel: &str,
    snake: &str,
) -> Result<(), BackendError> {
    if let Some(value) = dual_field(args, camel, snake) {
        if !value.is_array() {
            return Err(invalid(camel));
        }
        object.insert(snake.to_string(), value.clone());
    }
    Ok(())
}

fn update_optional_trimmed(
    object: &mut Map<String, Value>,
    args: &Value,
    camel: &str,
    snake: &str,
) -> Result<(), BackendError> {
    if let Some(value) = dual_field(args, camel, snake) {
        let value = value.as_str().ok_or_else(|| invalid(camel))?.trim();
        if value.is_empty() {
            object.remove(snake);
        } else {
            object.insert(snake.to_string(), Value::String(value.to_string()));
        }
    }
    Ok(())
}

fn update_nullable_string(
    object: &mut Map<String, Value>,
    args: &Value,
    camel: &str,
    snake: &str,
    none_sentinel: bool,
) -> Result<(), BackendError> {
    if let Some(value) = dual_field(args, camel, snake) {
        if value.is_null() {
            object.remove(snake);
        } else {
            let value = value.as_str().ok_or_else(|| invalid(camel))?;
            if none_sentinel && value == "__none__" {
                object.remove(snake);
            } else {
                object.insert(snake.to_string(), Value::String(value.to_string()));
            }
        }
    }
    Ok(())
}

fn update_nullable_value(
    object: &mut Map<String, Value>,
    args: &Value,
    camel: &str,
    snake: &str,
) -> Result<(), BackendError> {
    if let Some(value) = dual_field(args, camel, snake) {
        if value.is_null() {
            object.remove(snake);
        } else {
            object.insert(snake.to_string(), value.clone());
        }
    }
    Ok(())
}

fn dual_field<'a>(args: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    args.get(camel).or_else(|| args.get(snake))
}

fn optional_string_value<'a>(
    args: &'a Value,
    camel: &str,
    snake: &str,
) -> Result<Option<&'a str>, BackendError> {
    match dual_field(args, camel, snake) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or_else(|| invalid(camel)),
    }
}

fn optional_u32_value(args: &Value, camel: &str, snake: &str) -> Result<Option<u32>, BackendError> {
    match dual_field(args, camel, snake) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .and_then(|value| u32::try_from(value).ok())
            .map(Some)
            .ok_or_else(|| invalid(camel)),
    }
}

fn string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn u32_value(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

fn parse_porcelain_files(porcelain: &str) -> Vec<(String, String)> {
    porcelain
        .lines()
        .filter_map(|line| {
            if line.len() < 4 {
                return None;
            }
            let status = line[..2].trim().to_string();
            let path = line[3..]
                .rsplit_once(" -> ")
                .map(|(_, path)| path)
                .unwrap_or(&line[3..])
                .trim_matches('"')
                .to_string();
            Some((status, path))
        })
        .collect()
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn object_mut(value: &mut Value) -> Result<&mut Map<String, Value>, BackendError> {
    value
        .as_object_mut()
        .ok_or_else(|| BackendError::new(BackendErrorCode::Internal, "Invalid project data"))
}

fn invalid(field: impl Into<String>) -> BackendError {
    let field = field.into();
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("Invalid field '{field}'"),
    )
}

fn not_found(kind: &str, id: &str) -> BackendError {
    BackendError::new(
        BackendErrorCode::InvalidArgument,
        format!("{kind} not found: {id}"),
    )
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn sanitize_branch_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '/' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '/', '.'])
        .to_string()
}

fn sanitize_folder_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn worktree_value(
    id: &str,
    project_id: &str,
    name: &str,
    path: &str,
    branch: &str,
    created_at: u64,
    order: u64,
    contexts: &WorktreeContexts,
    setup: Option<&(String, String, bool)>,
) -> Value {
    serde_json::json!({
        "id":id,
        "project_id":project_id,
        "name":name,
        "path":path,
        "branch":branch,
        "base_branch":branch,
        "created_at":created_at,
        "setup_output":setup.map(|value| value.0.clone()),
        "setup_script":setup.map(|value| value.1.clone()),
        "setup_success":setup.map(|value| value.2),
        "session_type":"worktree",
        "pr_number":contexts.pull_request.as_ref().map(|context| context.number),
        "issue_number":contexts.issue.as_ref().map(|context| context.number),
        "linear_issue_identifier":contexts.linear.as_ref().map(|context| context.identifier.clone()),
        "security_alert_number":contexts.security.as_ref().map(|context| context.number),
        "security_alert_url":contexts.security.as_ref().and_then(|context| context.html_url.clone()),
        "advisory_ghsa_id":contexts.advisory.as_ref().map(|context| context.ghsa_id.clone()),
        "advisory_url":contexts.advisory.as_ref().and_then(|context| context.html_url.clone()),
        "order":order,
        "origin":Value::Null,
        "labels":[],
    })
}

fn unique_suffix_name(
    name: &str,
    project_path: &str,
    project_id: &str,
    snapshot: &ProjectsSnapshot,
    git: GitService,
) -> String {
    loop {
        let suffix = &Uuid::new_v4().simple().to_string()[..4];
        let candidate = format!("{name}-{suffix}");
        let stored = snapshot.worktrees.iter().any(|worktree| {
            string(worktree, "project_id") == Some(project_id)
                && string(worktree, "name") == Some(candidate.as_str())
        });
        if !stored && !git.branch_exists(project_path, &candidate) {
            return candidate;
        }
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

fn remove_file_if_present(path: &Path) {
    if path.exists() {
        if let Err(error) = std::fs::remove_file(path) {
            log::warn!("Failed to remove {}: {error}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ResolvedAppPaths, SessionService};
    use std::fs;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingEvents(Mutex<Vec<(String, Value)>>);

    impl EventSink for RecordingEvents {
        fn emit_json(&self, event: &str, payload: Value) -> Result<(), BackendError> {
            self.0.lock().unwrap().push((event.to_string(), payload));
            Ok(())
        }
    }

    fn service(temp: &tempfile::TempDir) -> ProjectService {
        let persistence = Arc::new(PersistenceService::new(Arc::new(ResolvedAppPaths::new(
            temp.path().join("data"),
            temp.path().join("config"),
            temp.path().join("cache"),
            temp.path().join("resources"),
        ))));
        ProjectService::new(persistence)
    }

    #[test]
    fn add_update_reorder_and_remove_projects() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["init", "-b", "main"])
            .output()
            .unwrap()
            .status
            .success());
        let service = service(&temp);
        let project = service.add(repo.display().to_string(), None).unwrap();
        let id = string(&project, "id").unwrap().to_string();
        assert_eq!(project["name"], "repo");
        assert_eq!(project["default_branch"], "main");

        let updated = service.update(&id, &serde_json::json!({"name":" Renamed ","future_arg":true,"defaultBackend":"__none__"})).unwrap();
        assert_eq!(updated["name"], "Renamed");
        assert!(updated.get("default_backend").is_none());
        service.reorder(std::slice::from_ref(&id)).unwrap();
        service.remove(&id).unwrap();
        assert!(service.list().unwrap().is_empty());
    }

    #[test]
    fn filters_archived_worktrees_and_blocks_active_removal() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({"id":"p1","linked_project_ids":[]})],
                worktrees: vec![
                    serde_json::json!({"id":"active","project_id":"p1"}),
                    serde_json::json!({"id":"archived","project_id":"p1","archived_at":42}),
                ],
                extra: Map::new(),
            })
            .unwrap();
        assert_eq!(service.list_worktrees("p1").unwrap().len(), 1);
        assert!(service.remove("p1").is_err());
    }

    #[test]
    fn linked_projects_are_kept_bidirectional() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({"id":"a"}), serde_json::json!({"id":"b"})],
                worktrees: vec![],
                extra: Map::new(),
            })
            .unwrap();
        service
            .update("a", &serde_json::json!({"linkedProjectIds":["b","b","a"]}))
            .unwrap();
        let projects = service.list().unwrap();
        assert_eq!(projects[0]["linked_project_ids"], serde_json::json!(["b"]));
        assert_eq!(projects[1]["linked_project_ids"], serde_json::json!(["a"]));
    }

    #[test]
    fn reorder_worktrees_keeps_base_first_without_consuming_an_order() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({"id":"p1"})],
                worktrees: vec![
                    serde_json::json!({"id":"base","project_id":"p1","session_type":"base","order":0}),
                    serde_json::json!({"id":"one","project_id":"p1","session_type":"worktree","order":2}),
                    serde_json::json!({"id":"two","project_id":"p1","session_type":"worktree","order":1}),
                ],
                extra: Map::new(),
            })
            .unwrap();

        service
            .reorder_worktrees(
                "p1",
                &["base".to_string(), "one".to_string(), "two".to_string()],
            )
            .unwrap();
        let snapshot = service.persistence.load_projects().unwrap();
        assert_eq!(snapshot.worktrees[0]["order"], 0);
        assert_eq!(snapshot.worktrees[1]["order"], 1);
        assert_eq!(snapshot.worktrees[2]["order"], 2);
    }

    #[test]
    fn porcelain_summary_preserves_rename_destinations() {
        let parsed = parse_porcelain_files(" M src/lib.rs\nR  old.rs -> new.rs\n?? scratch.txt\n");
        assert_eq!(
            parsed,
            vec![
                ("M".to_string(), "src/lib.rs".to_string()),
                ("R".to_string(), "new.rs".to_string()),
                ("??".to_string(), "scratch.txt".to_string()),
            ]
        );
    }

    #[test]
    fn base_session_archive_close_and_reopen_preserves_sessions() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1","name":"repo","path":temp.path(),"default_branch":"main"
                })],
                worktrees: vec![],
                extra: Map::new(),
            })
            .unwrap();
        let (base, created) = service.create_base_session("p1").unwrap();
        assert!(created);
        let base_id = string(&base, "id").unwrap();
        let sessions = SessionService::new(service.persistence.clone());
        let session = sessions
            .create(base_id, Some("Build"), None, None, None, None, None, None)
            .unwrap();
        let session_id = string(&session, "id").unwrap().to_string();
        let events = RecordingEvents::default();
        service
            .close_base_session(base_id, BaseSessionCloseMode::Archive, &events)
            .unwrap();
        assert!(service.get_worktree(base_id).is_err());
        assert!(service
            .persistence
            .base_session_index_path("p1")
            .unwrap()
            .exists());
        assert!(service
            .persistence
            .load_session_metadata(&session_id)
            .unwrap()
            .unwrap()["archived_by_base_close"]
            .as_bool()
            .unwrap());

        let (reopened, created) = service.create_base_session("p1").unwrap();
        assert!(created);
        let reopened_id = string(&reopened, "id").unwrap();
        assert!(service
            .persistence
            .load_session_index(reopened_id)
            .unwrap()
            .is_some());
        let metadata = service
            .persistence
            .load_session_metadata(&session_id)
            .unwrap()
            .unwrap();
        assert!(metadata.get("archived_at").is_none());
        assert!(metadata.get("archived_by_base_close").is_none());
        assert_eq!(events.0.lock().unwrap()[0].0, "worktree:deleted");
    }

    #[test]
    fn worktree_archive_round_trip_emits_shared_events() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        let path = temp.path().join("worktree");
        fs::create_dir(&path).unwrap();
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({"id":"p1"})],
                worktrees: vec![serde_json::json!({
                    "id":"w1","project_id":"p1","path":path,"session_type":"worktree"
                })],
                extra: Map::new(),
            })
            .unwrap();
        let events = RecordingEvents::default();
        service.archive_worktree("w1", &events).unwrap();
        assert_eq!(service.list_archived_worktrees().unwrap().len(), 1);
        let restored = service.unarchive_worktree("w1", &events).unwrap();
        assert!(restored.get("archived_at").is_none());
        let names = events
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "worktree:archived".to_string(),
                "worktree:unarchived".to_string()
            ]
        );
    }

    #[test]
    fn worktree_creation_events_match_the_browser_contract() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git_init_with_commit(&repo);
        #[cfg(unix)]
        let setup_script = "printf setup-complete";
        #[cfg(windows)]
        let setup_script = "Write-Output setup-complete";
        fs::write(
            repo.join("jean.json"),
            serde_json::to_vec(&serde_json::json!({
                "scripts":{"setup":setup_script}
            }))
            .unwrap(),
        )
        .unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1",
                    "name":"repo",
                    "path":repo,
                    "default_branch":"main",
                    "worktrees_dir":temp.path().join("worktrees")
                })],
                worktrees: vec![],
                extra: Map::new(),
            })
            .unwrap();
        let events = RecordingEvents::default();
        let worktree = service
            .create_worktree(
                "p1",
                Some("main"),
                Some("feature/event-contract"),
                Some("manual"),
                &events,
            )
            .unwrap();
        let recorded = events.0.lock().unwrap();
        assert_eq!(recorded.len(), 3);
        assert_eq!(recorded[0].0, "worktree:creating");
        assert_eq!(recorded[0].1["projectId"], "p1");
        assert_eq!(recorded[0].1["autoOpenInJean"], true);
        assert!(recorded[0].1.get("project_id").is_none());
        assert_eq!(recorded[1].0, "worktree:created");
        assert_eq!(recorded[1].1["worktree"]["id"], worktree["id"]);
        assert_eq!(recorded[1].1["autoOpenInJean"], true);
        assert_eq!(recorded[2].0, "worktree:setup_complete");
        assert_eq!(recorded[2].1["setup_output"], "setup-complete");
        assert_eq!(recorded[2].1["setup_success"], true);
        assert_eq!(worktree["setup_success"], true);
    }

    #[test]
    fn imported_worktree_can_be_archived_and_permanently_deleted() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        let imported_path = temp.path().join("imported");
        fs::create_dir(&repo).unwrap();
        git_init_with_commit(&repo);
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args([
                "worktree",
                "add",
                "-b",
                "feature/imported",
                imported_path.to_str().unwrap(),
                "main"
            ])
            .output()
            .unwrap()
            .status
            .success());
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1","name":"repo","path":repo,"default_branch":"main"
                })],
                worktrees: vec![],
                extra: Map::new(),
            })
            .unwrap();
        let events = RecordingEvents::default();
        let imported = service
            .import_worktree("p1", imported_path.to_str().unwrap(), &events)
            .unwrap();
        let worktree_id = string(&imported, "id").unwrap();
        assert_eq!(imported["branch"], "feature/imported");
        assert_eq!(events.0.lock().unwrap()[0].1["worktree"]["id"], worktree_id);
        assert!(service
            .import_worktree("p1", imported_path.to_str().unwrap(), &events)
            .is_err());
        assert!(service
            .permanently_delete_worktree(worktree_id, &events)
            .is_err());

        let session = SessionService::new(service.persistence.clone())
            .create(
                worktree_id,
                Some("Imported session"),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let session_id = string(&session, "id").unwrap();
        let combined_dir = temp.path().join("data/combined-contexts");
        fs::create_dir_all(&combined_dir).unwrap();
        let combined = combined_dir.join(format!("{session_id}-combined.md"));
        fs::write(&combined, "context").unwrap();

        service.archive_worktree(worktree_id, &events).unwrap();
        service
            .permanently_delete_worktree(worktree_id, &events)
            .unwrap();
        assert!(service.get_worktree(worktree_id).is_err());
        assert!(!imported_path.exists());
        assert!(!combined.exists());
        assert!(service
            .persistence
            .load_session_index(worktree_id)
            .unwrap()
            .is_none());
        let recorded = events.0.lock().unwrap();
        assert_eq!(recorded.last().unwrap().0, "worktree:permanently_deleted");
        assert_eq!(recorded.last().unwrap().1["project_id"], "p1");
    }

    #[test]
    fn worktree_metadata_mutations_are_shared_and_preserve_partial_status() {
        let temp = tempfile::tempdir().unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({"id":"p1"})],
                worktrees: vec![
                    serde_json::json!({
                        "id":"base","project_id":"p1","name":"main","branch":"main",
                        "session_type":"base","cached_ahead_count":7,"label":{"name":"legacy"}
                    }),
                    serde_json::json!({
                        "id":"other","project_id":"p1","name":"Existing","branch":"existing",
                        "session_type":"worktree"
                    }),
                ],
                extra: Map::new(),
            })
            .unwrap();

        let renamed = service.rename_worktree("base", "  Display name  ").unwrap();
        assert_eq!(renamed["name"], "Display name");
        assert!(service.rename_worktree("base", "Existing").is_err());
        service
            .update_worktree_labels(
                "base",
                vec![
                    serde_json::json!({"name":"Urgent","color":"#f00"}),
                    serde_json::json!({"name":"urgent","color":"#0f0"}),
                    serde_json::json!({"name":"Review","color":"#00f","pinned":true}),
                ],
            )
            .unwrap();
        service
            .update_worktree_cached_status(
                "base",
                &serde_json::json!({
                    "branch":"develop",
                    "prStatus":"open",
                    "behind_count":3
                }),
            )
            .unwrap();
        service.set_worktree_last_opened("base").unwrap();

        let worktree = service.get_worktree("base").unwrap();
        assert_eq!(worktree["branch"], "develop");
        assert_eq!(worktree["name"], "develop");
        assert_eq!(worktree["cached_pr_status"], "open");
        assert_eq!(worktree["cached_behind_count"], 3);
        assert_eq!(worktree["cached_ahead_count"], 7);
        assert_eq!(worktree["labels"].as_array().unwrap().len(), 2);
        assert_eq!(worktree["labels"][0]["name"], "Urgent");
        assert!(worktree.get("label").is_none());
        assert!(worktree["cached_status_at"].as_u64().unwrap() > 0);
        assert!(worktree["last_opened_at"].as_u64().unwrap() > 0);
    }

    #[test]
    fn existing_branch_creation_runs_entire_core_pipeline() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        fs::create_dir(&repo).unwrap();
        git_init_with_commit(&repo);
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args(["branch", "feature/existing"])
            .output()
            .unwrap()
            .status
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(&repo)
            .args([
                "remote",
                "add",
                "origin",
                "git@github.com:acme/pipeline.git"
            ])
            .output()
            .unwrap()
            .status
            .success());
        #[cfg(unix)]
        let setup_script = "printf rich-setup";
        #[cfg(windows)]
        let setup_script = "Write-Output rich-setup";
        fs::write(
            repo.join("jean.json"),
            serde_json::to_vec(&serde_json::json!({"scripts":{"setup":setup_script}})).unwrap(),
        )
        .unwrap();
        let service = service(&temp);
        service
            .persistence
            .save_projects(&ProjectsSnapshot {
                projects: vec![serde_json::json!({
                    "id":"p1","name":"Repo","path":repo,
                    "default_branch":"main","worktrees_dir":temp.path().join("worktrees")
                })],
                worktrees: vec![],
                extra: Map::new(),
            })
            .unwrap();
        let events = RecordingEvents::default();
        let (pending, task) = service
            .prepare_existing_branch_worktree(
                ExistingBranchWorktreeInput {
                    project_id: "p1".to_string(),
                    branch_name: "feature/existing".to_string(),
                    contexts: WorktreeContexts {
                        issue: Some(crate::IssueContext {
                            number: 7,
                            title: "Shared pipeline".to_string(),
                            body: None,
                            comments: vec![],
                        }),
                        linear: Some(crate::LinearIssueContext {
                            id: "linear-7".to_string(),
                            identifier: "ENG-7".to_string(),
                            title: "Shared linear context".to_string(),
                            description: None,
                            comments: vec![],
                        }),
                        ..WorktreeContexts::default()
                    },
                    auto_open_in_jean: false,
                },
                &events,
            )
            .unwrap();
        assert_eq!(pending["order"], 0);
        assert_eq!(pending["issue_number"], 7);
        assert_eq!(events.0.lock().unwrap()[0].1["projectId"], "p1");
        let completed = service
            .complete_existing_branch_worktree(task, &events)
            .unwrap();
        assert_eq!(completed["branch"], "feature/existing");
        assert_eq!(completed["setup_output"], "rich-setup");
        assert_eq!(completed["setup_success"], true);
        assert!(Path::new(completed["path"].as_str().unwrap()).exists());
        let contexts_dir = service.persistence.git_contexts_dir().unwrap();
        assert!(contexts_dir.join("acme-pipeline-issue-7.md").exists());
        assert!(contexts_dir.join("Repo-linear-eng-7.md").exists());
        let recorded = events.0.lock().unwrap();
        assert_eq!(recorded.last().unwrap().0, "worktree:created");
        assert_eq!(recorded.last().unwrap().1["autoOpenInJean"], false);
    }

    fn git_init_with_commit(repo: &Path) {
        assert!(std::process::Command::new("git")
            .current_dir(repo)
            .args(["init", "-b", "main"])
            .output()
            .unwrap()
            .status
            .success());
        for (key, value) in [
            ("user.name", "Jean Core Tests"),
            ("user.email", "jean-core@example.test"),
        ] {
            assert!(std::process::Command::new("git")
                .current_dir(repo)
                .args(["config", key, value])
                .output()
                .unwrap()
                .status
                .success());
        }
        fs::write(repo.join("README.md"), "test").unwrap();
        assert!(std::process::Command::new("git")
            .current_dir(repo)
            .args(["add", "README.md"])
            .output()
            .unwrap()
            .status
            .success());
        assert!(std::process::Command::new("git")
            .current_dir(repo)
            .args(["commit", "-m", "initial"])
            .output()
            .unwrap()
            .status
            .success());
    }
}
