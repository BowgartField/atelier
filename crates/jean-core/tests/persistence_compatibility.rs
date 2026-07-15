use jean_core::{PersistenceService, ResolvedAppPaths};
use serde_json::Value;
use std::fs;
use std::sync::Arc;

fn fixture(name: &str) -> &'static str {
    match name {
        "preferences" => include_str!("fixtures/preferences-existing.json"),
        "ui-state" => include_str!("fixtures/ui-state-existing.json"),
        "projects" => include_str!("fixtures/projects-existing.json"),
        _ => panic!("unknown fixture"),
    }
}

fn store(temp: &tempfile::TempDir) -> PersistenceService {
    PersistenceService::new(Arc::new(ResolvedAppPaths::new(
        temp.path().to_path_buf(),
        temp.path().join("config"),
        temp.path().join("cache"),
        temp.path().join("resources"),
    )))
}

#[test]
fn existing_preferences_and_ui_state_round_trip_without_field_loss() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    fs::write(store.preferences_path().unwrap(), fixture("preferences")).unwrap();
    fs::write(store.ui_state_path().unwrap(), fixture("ui-state")).unwrap();

    let preferences = store.load_preferences().unwrap();
    let ui_state = store.load_ui_state().unwrap();
    store.save_preferences(&preferences).unwrap();
    store.save_ui_state(&ui_state).unwrap();

    let saved_preferences: Value =
        serde_json::from_str(&fs::read_to_string(store.preferences_path().unwrap()).unwrap())
            .unwrap();
    let saved_ui_state: Value =
        serde_json::from_str(&fs::read_to_string(store.ui_state_path().unwrap()).unwrap()).unwrap();
    assert_eq!(saved_preferences, preferences);
    assert_eq!(
        saved_preferences["future_nested_field"]["must_survive"],
        true
    );
    assert_eq!(saved_ui_state, ui_state);
    assert_eq!(
        saved_ui_state["future_ui_field"],
        serde_json::json!([1, 2, 3])
    );
}

#[test]
fn existing_projects_round_trip_without_field_loss() {
    let temp = tempfile::tempdir().unwrap();
    let store = store(&temp);
    fs::write(store.projects_path().unwrap(), fixture("projects")).unwrap();

    let projects = store.load_projects().unwrap();
    store.save_projects(&projects).unwrap();
    let saved: Value =
        serde_json::from_str(&fs::read_to_string(store.projects_path().unwrap()).unwrap()).unwrap();

    assert_eq!(saved["projects"][0]["future_project_field"], "preserved");
    assert_eq!(saved["worktrees"][0]["future_worktree_field"], 42);
    assert_eq!(saved["future_top_level_field"], "preserved");
}
