use serde::{Deserialize, Serialize};

/// Event payload for page load events (started/finished)
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPageLoadEvent {
    pub tab_id: String,
    pub url: String,
}

/// Event payload for navigation state change (after navigate/back/forward/load)
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserNavEvent {
    pub tab_id: String,
    pub url: String,
}

/// Event payload for title change
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserTitleEvent {
    pub tab_id: String,
    pub title: String,
}

/// Event payload for tab close
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserClosedEvent {
    pub tab_id: String,
}
