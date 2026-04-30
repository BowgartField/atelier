/**
 * Types for the embedded browser pane (per-worktree multi-tab native webview).
 * Mirror Rust event payload shapes from src-tauri/src/browser/types.rs
 */

export type ModalBrowserDockMode = 'floating' | 'left' | 'right' | 'bottom'

export interface BrowserTab {
  id: string
  worktreeId: string
  url: string
  title: string
  isLoading: boolean
  /** Set when load fails / times out. Cleared on next successful navigation. */
  error?: string | null
  /** What the user asked to load. Set in navigate(); cleared on browser:loaded resolution. */
  requestedUrl?: string | null
  /** Last URL that successfully reached browser:loaded. Used to detect WKWebView fallback-to-previous on failed nav. */
  lastLoadedUrl?: string | null
}

// Rust → React event payloads (camelCase via serde rename_all)
export interface BrowserPageLoadEvent {
  tabId: string
  url: string
}

export interface BrowserNavEvent {
  tabId: string
  url: string
}

export interface BrowserTitleEvent {
  tabId: string
  title: string
}

export interface BrowserClosedEvent {
  tabId: string
}

/** Bounds passed to browser_create / browser_set_bounds (logical pixels). */
export interface BrowserBounds {
  x: number
  y: number
  width: number
  height: number
}
