use tauri::webview::{PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Url, WebviewUrl};

use super::registry::{
    get_all_tab_ids, get_label, has_tab, label_for_tab, register_tab, unregister_tab,
};
use super::types::{BrowserClosedEvent, BrowserNavEvent, BrowserPageLoadEvent, BrowserTitleEvent};
use crate::http_server::EmitExt;

const MAIN_WINDOW: &str = "main";

/// JS injected after every page load. Reports title changes back to Rust via
/// the `browser_report_title` command. Tauri v2 has no first-class title-change
/// event for child webviews.
fn title_observer_script(tab_id: &str) -> String {
    let escaped = tab_id.replace('\\', "\\\\").replace('\'', "\\'");
    format!(
        r#"(function() {{
            const tabId = '{escaped}';
            const internals = window.__TAURI_INTERNALS__;
            if (!internals || !internals.invoke) return;
            const report = () => {{
                try {{ internals.invoke('browser_report_title', {{ tabId, title: document.title || '' }}); }}
                catch (_) {{}}
            }};
            report();
            try {{
                const target = document.querySelector('title') || document.head;
                if (target && window.MutationObserver) {{
                    const obs = new MutationObserver(report);
                    obs.observe(target, {{ subtree: true, characterData: true, childList: true }});
                }}
            }} catch (_) {{}}
        }})();"#
    )
}

/// Create a new browser tab as a child Webview of the main window.
#[tauri::command]
pub async fn browser_create(
    app: AppHandle,
    tab_id: String,
    url: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<String, String> {
    log::trace!("browser_create tab_id={tab_id} url={url}");

    if has_tab(&tab_id) {
        return Err(format!("Browser tab '{tab_id}' already exists"));
    }

    let parsed = Url::parse(&url).map_err(|e| format!("invalid url: {e}"))?;
    let label = label_for_tab(&tab_id);

    let main = app
        .get_window(MAIN_WINDOW)
        .ok_or_else(|| "main window not found".to_string())?;

    let app_for_load = app.clone();
    let tab_for_load = tab_id.clone();
    let builder = WebviewBuilder::new(&label, WebviewUrl::External(parsed)).on_page_load(
        move |webview, payload| {
            let url_str = payload.url().to_string();
            let event_name = match payload.event() {
                PageLoadEvent::Started => "browser:loading",
                PageLoadEvent::Finished => "browser:loaded",
            };
            let _ = app_for_load.emit_all(
                event_name,
                &BrowserPageLoadEvent {
                    tab_id: tab_for_load.clone(),
                    url: url_str.clone(),
                },
            );
            let _ = app_for_load.emit_all(
                "browser:nav",
                &BrowserNavEvent {
                    tab_id: tab_for_load.clone(),
                    url: url_str,
                },
            );
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = webview.eval(title_observer_script(&tab_for_load));
            }
        },
    );

    // Frontend sends PHYSICAL pixels (CSS px × devicePixelRatio).
    // Use PhysicalPosition/PhysicalSize so Tauri stores them as-is — bypassing
    // its scale_factor() conversion, which can disagree with WKWebView's real
    // devicePixelRatio under fractional macOS display scaling.
    main.add_child(
        builder,
        PhysicalPosition::new(x as i32, y as i32),
        PhysicalSize::new((width.max(1.0)) as u32, (height.max(1.0)) as u32),
    )
    .map_err(|e| format!("failed to add child webview: {e}"))?;

    register_tab(tab_id, label.clone());
    Ok(label)
}

#[tauri::command]
pub async fn browser_navigate(app: AppHandle, tab_id: String, url: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    let parsed = Url::parse(&url).map_err(|e| format!("invalid url: {e}"))?;
    webview.navigate(parsed).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_back(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview
        .eval("history.back()".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_forward(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview
        .eval("history.forward()".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview
        .eval("location.reload()".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_stop(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview
        .eval("window.stop()".to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_set_bounds(
    app: AppHandle,
    tab_id: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    // Frontend sends PHYSICAL pixels — see browser_create for rationale.
    webview
        .set_position(PhysicalPosition::new(x as i32, y as i32))
        .map_err(|e| e.to_string())?;
    webview
        .set_size(PhysicalSize::new(
            (width.max(1.0)) as u32,
            (height.max(1.0)) as u32,
        ))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_set_visible(
    app: AppHandle,
    tab_id: String,
    visible: bool,
) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    if visible {
        webview.show().map_err(|e| e.to_string())
    } else {
        webview.hide().map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn browser_set_focus(app: AppHandle, tab_id: String) -> Result<(), String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview.set_focus().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_get_url(app: AppHandle, tab_id: String) -> Result<String, String> {
    let label = get_label(&tab_id).ok_or_else(|| format!("tab '{tab_id}' not found"))?;
    let webview = app
        .get_webview(&label)
        .ok_or_else(|| format!("webview '{label}' not found"))?;
    webview
        .url()
        .map(|u| u.to_string())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_close(app: AppHandle, tab_id: String) -> Result<(), String> {
    log::trace!("browser_close tab_id={tab_id}");
    if let Some(label) = unregister_tab(&tab_id) {
        if let Some(webview) = app.get_webview(&label) {
            let _ = webview.close();
        }
    }
    let _ = app.emit_all("browser:closed", &BrowserClosedEvent { tab_id });
    Ok(())
}

#[tauri::command]
pub async fn get_active_browser_tabs() -> Vec<String> {
    get_all_tab_ids()
}

#[tauri::command]
pub async fn has_active_browser_tab(tab_id: String) -> bool {
    has_tab(&tab_id)
}

/// Called by an injected MutationObserver every time the page <title> changes.
#[tauri::command]
pub async fn browser_report_title(
    app: AppHandle,
    tab_id: String,
    title: String,
) -> Result<(), String> {
    app.emit_all("browser:title", &BrowserTitleEvent { tab_id, title })
}
