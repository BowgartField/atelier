use crate::{
    AppPaths, ChatRunManager, EventSink, PersistenceService, TerminalManager, WsBroadcaster,
};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct ResourceRegistry {
    ids: RwLock<HashSet<String>>,
}

impl ResourceRegistry {
    pub fn register(&self, id: impl Into<String>) -> bool {
        self.ids
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id.into())
    }

    pub fn unregister(&self, id: &str) -> bool {
        self.ids
            .write()
            .unwrap_or_else(|error| error.into_inner())
            .remove(id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.ids
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .contains(id)
    }

    pub fn len(&self) -> usize {
        self.ids
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub struct BackendState {
    pub websocket: Arc<WsBroadcaster>,
    pub background_tasks: Arc<ResourceRegistry>,
    pub terminals: Arc<TerminalManager>,
    pub chat_runs: Arc<ChatRunManager>,
    pub tunnels: Arc<ResourceRegistry>,
    pub shutdown: CancellationToken,
}

impl BackendState {
    pub fn new(websocket: Arc<WsBroadcaster>) -> Self {
        Self {
            websocket,
            background_tasks: Arc::new(ResourceRegistry::default()),
            terminals: Arc::new(TerminalManager::default()),
            chat_runs: Arc::new(ChatRunManager::default()),
            tunnels: Arc::new(ResourceRegistry::default()),
            shutdown: CancellationToken::new(),
        }
    }
}

#[derive(Clone)]
pub struct BackendContext {
    pub paths: Arc<dyn AppPaths>,
    pub events: Arc<dyn EventSink>,
    pub persistence: Arc<PersistenceService>,
    pub state: Arc<BackendState>,
}

impl BackendContext {
    pub fn new(
        paths: Arc<dyn AppPaths>,
        events: Arc<dyn EventSink>,
        state: Arc<BackendState>,
    ) -> Self {
        let persistence = Arc::new(PersistenceService::new(paths.clone()));
        Self {
            paths,
            events,
            persistence,
            state,
        }
    }

    pub fn shutdown(&self) {
        self.state.terminals.kill_all();
        self.state.chat_runs.cancel_all();
        self.state.shutdown.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registries_are_explicit_and_idempotent() {
        let registry = ResourceRegistry::default();
        assert!(registry.register("run-1"));
        assert!(!registry.register("run-1"));
        assert!(registry.contains("run-1"));
        assert_eq!(registry.len(), 1);
        assert!(registry.unregister("run-1"));
        assert!(registry.is_empty());
    }
}
