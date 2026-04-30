/**
 * Module-level storage for xterm.js Terminal instances.
 *
 * This decouples terminal lifecycle from React component lifecycle.
 * Terminals persist across component mount/unmount cycles, preserving
 * buffer content, cursor position, and running processes.
 *
 * Only disposed when user explicitly closes the terminal.
 */

import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { openExternal } from '@/lib/platform'
import {
  invoke,
  isTransportConnected,
  subscribeTransportStatus,
} from '@/lib/transport'
import { listen } from '@/lib/transport'
import { useTerminalStore } from '@/store/terminal-store'
import type {
  TerminalOutputEvent,
  TerminalStartedEvent,
  TerminalStoppedEvent,
} from '@/types/terminal'

interface PersistentTerminal {
  terminal: Terminal
  fitAddon: FitAddon
  /** Promises that resolve to UnlistenFn — kept as promises to close the
   *  registration race: if disposeTerminal runs before listen() resolves,
   *  we still await and call the returned unlisten. */
  listeners: Promise<() => void>[]
  worktreeId: string
  worktreePath: string
  command: string | null
  commandArgs: string[] | null
  initialized: boolean // PTY has been started
  onStopped?: (exitCode: number | null, signal: string | null) => void
}

// Module-level Map - persists across React mount/unmount cycles
const instances = new Map<string, PersistentTerminal>()

/** Register one document/window wake handler that forces all xterm instances
 *  to repaint when the webview resumes from idle/sleep (issue #320).
 *  RAF-based DOM renderer can stall after macOS App Nap or DPMS sleep;
 *  terminal.refresh() kicks the render queue without needing a new frame. */
let wakeHandlerRegistered = false
function ensureWakeHandler(): void {
  if (wakeHandlerRegistered) return
  wakeHandlerRegistered = true
  const wake = () => {
    if (document.visibilityState !== 'visible') return
    for (const inst of instances.values()) {
      try {
        inst.terminal.refresh(0, Math.max(0, inst.terminal.rows - 1))
      } catch {
        // ignore — terminal may be in mid-dispose
      }
    }
  }
  document.addEventListener('visibilitychange', wake)
  window.addEventListener('focus', wake)
}

/** Register one transport status subscriber that writes a [Reconnecting...]/
 *  [Reconnected] banner into every live xterm instance on connection-state
 *  transitions. Web access mode only — native always reports connected.
 *  Helps users understand why their input is being dropped during outages. */
let transportStatusSubscribed = false
let lastTransportConnected: boolean | null = null
function ensureTransportStatusBanner(): void {
  if (transportStatusSubscribed) return
  transportStatusSubscribed = true
  lastTransportConnected = isTransportConnected()
  subscribeTransportStatus(() => {
    const connected = isTransportConnected()
    if (connected === lastTransportConnected) return
    const message = connected
      ? '\r\n\x1b[32m[Reconnected]\x1b[0m\r\n'
      : '\r\n\x1b[33m[Reconnecting...]\x1b[0m\r\n'
    for (const inst of instances.values()) {
      try {
        inst.terminal.write(message)
      } catch {
        // ignore — terminal may be in mid-dispose
      }
    }
    lastTransportConnected = connected
  })
}

const FALLBACK_TERMINAL_BACKGROUND = '#101010'
const FALLBACK_TERMINAL_FOREGROUND = '#fafafa'
const FALLBACK_TERMINAL_SELECTION = '#242424'

function getRootColorVariable(name: string, fallback: string): string {
  if (typeof window === 'undefined' || typeof document === 'undefined') {
    return fallback
  }

  const value = getComputedStyle(document.documentElement)
    .getPropertyValue(name)
    .trim()

  return value || fallback
}

function getTerminalTheme() {
  const foreground = getRootColorVariable(
    '--card-foreground',
    FALLBACK_TERMINAL_FOREGROUND
  )

  return {
    background: getRootColorVariable(
      '--background',
      FALLBACK_TERMINAL_BACKGROUND
    ),
    foreground,
    cursor: foreground,
    selectionBackground: getRootColorVariable(
      '--muted',
      FALLBACK_TERMINAL_SELECTION
    ),
  }
}

// TODO: Add memory cap for detached terminals (e.g., 20 max)
// For now, typical usage won't hit memory limits

/**
 * Get existing terminal instance or create a new one.
 * Creates xterm.js Terminal, FitAddon, and event listeners.
 * Does NOT start PTY - that happens in attachToContainer when first attached.
 */
export function getOrCreateTerminal(
  terminalId: string,
  options: {
    worktreeId: string
    worktreePath: string
    command?: string | null
    commandArgs?: string[] | null
  }
): PersistentTerminal {
  const existing = instances.get(terminalId)
  if (existing) {
    existing.terminal.options.theme = getTerminalTheme()
    return existing
  }

  const {
    worktreeId,
    worktreePath,
    command = null,
    commandArgs = null,
  } = options
  const { setTerminalRunning } = useTerminalStore.getState()

  // Create xterm.js Terminal instance
  const terminal = new Terminal({
    cursorBlink: true,
    fontSize: 13,
    fontFamily:
      'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Monaco, Consolas, monospace',
    theme: getTerminalTheme(),
    allowProposedApi: true,
  })

  // Block most app shortcuts when terminal is focused. Only let specific CMD
  // combos bubble to the global handler for terminal-specific actions.
  // Ctrl+C/D/Z/L etc. always reach the PTY for terminal signal handling.
  terminal.attachCustomKeyEventHandler(event => {
    if (event.metaKey) {
      const code = event.code
      // CMD+` → toggle terminal panel
      if (code === 'Backquote') return false
      // CMD+T → new terminal tab
      if (!event.shiftKey && !event.altKey && code === 'KeyT') return false
      // CMD+W → close terminal tab
      if (!event.shiftKey && !event.altKey && code === 'KeyW') return false
      // CMD+1..9 → switch terminal tab
      if (!event.shiftKey && !event.altKey && /^Digit[1-9]$/.test(code)) {
        return false
      }
      // CMD+Alt+Backspace → cancel prompt
      if (event.altKey && (code === 'Backspace' || code === 'Delete'))
        return false
      // All other CMD shortcuts: xterm consumes them (prevents app actions)
      return true
    }
    return true
  })

  const fitAddon = new FitAddon()
  terminal.loadAddon(fitAddon)
  terminal.loadAddon(
    new WebLinksAddon((_event, uri) => {
      openExternal(uri)
    })
  )

  // Handle user input - forward to PTY.
  // Drop input while transport is disconnected: queueing 30s+ of keystrokes
  // and dumping them into the shell on reconnect = footgun (e.g. dangerous
  // partial commands executed). Banner makes the dropped state visible.
  terminal.onData(data => {
    if (!isTransportConnected()) return
    invoke('terminal_write', { terminalId, data }).catch(console.error)
  })

  // Ensure the visibility/focus wake handler is running.
  ensureWakeHandler()
  // Ensure transport status banner subscription is active (web access mode).
  ensureTransportStatusBanner()

  const listeners: Promise<() => void>[] = []

  // Setup event listeners ONCE when terminal is created.
  // Stored as Promise<UnlistenFn> (not resolved values) so that disposeTerminal
  // can await them even if disposal races the async listen() resolution.
  listeners.push(
    listen<TerminalOutputEvent>('terminal:output', event => {
      if (event.payload.terminal_id === terminalId) {
        terminal.write(event.payload.data)
      }
    })
  )

  listeners.push(
    listen<TerminalStartedEvent>('terminal:started', event => {
      if (event.payload.terminal_id === terminalId) {
        setTerminalRunning(terminalId, true)
      }
    })
  )

  listeners.push(
    listen<TerminalStoppedEvent>('terminal:stopped', event => {
      if (event.payload.terminal_id === terminalId) {
        setTerminalRunning(terminalId, false)
        const exitCode = event.payload.exit_code
        const signal = event.payload.signal
        const exitLabel =
          signal != null ? `signal ${signal}` : `code ${exitCode ?? 'unknown'}`
        terminal.writeln(
          `\r\n\x1b[90m[Process exited with ${exitLabel}]\x1b[0m`
        )
        const inst = instances.get(terminalId)
        inst?.onStopped?.(exitCode, signal)

        // Auto-close terminal tab on clean exit:
        // - code 0 — any terminal
        // - SIGINT (Ctrl+C) or SIGTERM (graceful stop) — user or system stop
        // SIGKILL, SIGSEGV, SIGABRT, etc. are NOT clean → mark as failed.
        const isRunTerminal = inst?.command != null
        const isIntentionalSignal =
          signal != null &&
          (signal.includes('Interrupt') || signal.includes('Terminated'))
        const isCleanExit = exitCode === 0 || isIntentionalSignal

        if (isCleanExit && inst) {
          const wId = inst.worktreeId
          setTimeout(() => {
            if (!instances.has(terminalId)) return // Already disposed
            // eslint-disable-next-line @typescript-eslint/no-empty-function
            invoke('stop_terminal', { terminalId }).catch(() => {})
            disposeTerminal(terminalId)
            const { removeTerminal, setTerminalPanelOpen } =
              useTerminalStore.getState()
            removeTerminal(wId, terminalId)
            const remaining = useTerminalStore.getState().terminals[wId] ?? []
            if (remaining.length === 0) {
              setTerminalPanelOpen(wId, false)
              useTerminalStore.getState().setTerminalVisible(false)
              useTerminalStore.getState().setModalTerminalOpen(wId, false)
            }
          }, 0)
        } else if (isRunTerminal) {
          // Non-zero exit on a run terminal → mark as failed (red indicator in sidebar)
          useTerminalStore.getState().setTerminalFailed(terminalId, true)
        }
      }
    })
  )

  const instance: PersistentTerminal = {
    terminal,
    fitAddon,
    listeners,
    worktreeId,
    worktreePath,
    command,
    commandArgs,
    initialized: false,
  }

  // Apply any pending onStopped callback registered before creation
  const pendingCb = pendingOnStopped.get(terminalId)
  if (pendingCb) {
    instance.onStopped = pendingCb
    pendingOnStopped.delete(terminalId)
  }

  instances.set(terminalId, instance)
  return instance
}

/**
 * Get terminal instance by ID.
 */
export function getInstance(
  terminalId: string
): PersistentTerminal | undefined {
  return instances.get(terminalId)
}

/**
 * Attach terminal to a DOM container.
 * If first attach, calls terminal.open(). Otherwise moves DOM element.
 * Starts PTY if not already initialized.
 */
export async function attachToContainer(
  terminalId: string,
  container: HTMLDivElement
): Promise<void> {
  const instance = instances.get(terminalId)
  if (!instance) {
    console.error(
      '[terminal-instances] attachToContainer: instance not found:',
      terminalId
    )
    return
  }

  const {
    terminal,
    fitAddon,
    worktreePath,
    command,
    commandArgs,
    initialized,
  } = instance
  const terminalElement = terminal.element

  terminal.options.theme = getTerminalTheme()

  if (!terminalElement) {
    // First attach - call open() to create DOM element
    terminal.open(container)
  } else if (terminalElement.parentNode !== container) {
    // Re-attach - move DOM element to new container
    container.appendChild(terminalElement)
  }

  // Fit terminal to container and start/reconnect PTY
  requestAnimationFrame(async () => {
    fitAddon.fit()
    // Enforce minimum dimensions — degenerate sizes (e.g. rows=0 during dialog
    // animation) cause portable_pty to crash with an internal assertion failure.
    const rawCols = terminal.cols
    const rawRows = terminal.rows
    const cols = rawCols < 2 ? 80 : rawCols
    const rows = rawRows < 2 ? 24 : rawRows
    console.log(
      `[terminal-instances] attachToContainer ${terminalId}: fit=${rawCols}x${rawRows} → used=${cols}x${rows}, initialized=${initialized}, container=${container.clientWidth}x${container.clientHeight}`
    )

    if (!initialized) {
      // First time - check if PTY already exists (reconnecting after app restart)
      const ptyExists = await invoke<boolean>('has_active_terminal', {
        terminalId,
      })

      if (ptyExists) {
        // PTY exists - just resize and mark as running
        useTerminalStore.getState().setTerminalRunning(terminalId, true)
        await invoke('terminal_resize', { terminalId, cols, rows }).catch(
          console.error
        )
      } else {
        // Start new PTY process
        await invoke('start_terminal', {
          terminalId,
          worktreePath,
          cols,
          rows,
          command,
          commandArgs,
        }).catch(error => {
          console.error('[terminal-instances] start_terminal failed:', error)
          terminal.writeln(`\x1b[31mFailed to start terminal: ${error}\x1b[0m`)
        })
      }

      instance.initialized = true
    } else {
      // Already initialized - just resize
      await invoke('terminal_resize', { terminalId, cols, rows }).catch(
        console.error
      )
    }

    terminal.focus()
  })
}

/**
 * Start a terminal PTY without attaching to DOM.
 * Creates the xterm instance (for event listeners + output buffering) and spawns
 * the PTY immediately. When the user later opens the session, attachToContainer
 * will detect the running PTY via has_active_terminal and reconnect.
 */
export function startHeadless(
  terminalId: string,
  options: {
    worktreeId: string
    worktreePath: string
    command: string
    commandArgs?: string[] | null
  }
): void {
  const instance = getOrCreateTerminal(terminalId, options)
  if (instance.initialized) return // Already started

  instance.initialized = true
  invoke('start_terminal', {
    terminalId,
    worktreePath: options.worktreePath,
    cols: 80,
    rows: 24,
    command: options.command,
    commandArgs: options.commandArgs ?? null,
  }).catch(error => {
    console.error('[terminal-instances] headless start_terminal failed:', error)
  })
}

/**
 * Detach terminal from DOM container.
 * Terminal stays in memory with preserved buffer.
 */
export function detachFromContainer(terminalId: string): void {
  const instance = instances.get(terminalId)
  if (!instance) return

  const terminalElement = instance.terminal.element
  if (terminalElement?.parentNode) {
    terminalElement.parentNode.removeChild(terminalElement)
  }
}

/**
 * Fit terminal to its container dimensions.
 */
export function fitTerminal(terminalId: string): void {
  const instance = instances.get(terminalId)
  if (!instance) return

  instance.fitAddon.fit()
  const { cols, rows } = instance.terminal
  invoke('terminal_resize', { terminalId, cols, rows }).catch(console.error)
}

/**
 * Focus terminal for keyboard input.
 */
export function focusTerminal(terminalId: string): void {
  const instance = instances.get(terminalId)
  if (!instance) return

  instance.terminal.focus()
}

/**
 * Dispose a single terminal instance.
 * Cleans up event listeners, disposes xterm, removes from Map.
 * Does NOT stop PTY - caller should do that separately.
 *
 * Async so it can await listen() promises that may not have resolved yet
 * (prevents orphan listeners when dispose races the initial listen() call).
 * All callers are fire-and-forget so the async signature is safe.
 */
export async function disposeTerminal(terminalId: string): Promise<void> {
  const instance = instances.get(terminalId)
  if (!instance) return

  // Remove from Map first so new lookups don't find a half-disposed instance
  instances.delete(terminalId)

  // Await each listener promise then call the returned unlisten function.
  // If listen() hasn't resolved yet this ensures we still unsubscribe.
  for (const listenerPromise of instance.listeners) {
    try {
      const unlisten = await listenerPromise
      unlisten()
    } catch {
      // listen() itself failed — nothing to unsubscribe
    }
  }

  // Dispose xterm.js (clears buffer, removes DOM)
  instance.terminal.dispose()
}

/**
 * Dispose all terminals for a worktree.
 * Used when worktree is deleted/archived/closed.
 * Stops PTY processes and cleans up xterm instances.
 */
export function disposeAllWorktreeTerminals(worktreeId: string): void {
  // Get terminal IDs from store and clear store state
  const terminalIds = useTerminalStore.getState().closeAllTerminals(worktreeId)

  // Dispose each terminal instance and stop PTY
  for (const terminalId of terminalIds) {
    // Stop PTY process
    invoke('stop_terminal', { terminalId }).catch(() => {
      // Terminal may already be stopped
    })

    // Dispose xterm instance
    disposeTerminal(terminalId)
  }
}

/**
 * Check if a terminal instance exists.
 */
export function hasInstance(terminalId: string): boolean {
  return instances.has(terminalId)
}

// Pending onStopped callbacks for terminals not yet created
const pendingOnStopped = new Map<
  string,
  (exitCode: number | null, signal: string | null) => void
>()

/**
 * Register a callback for when a terminal's process exits.
 * Can be called before or after terminal creation.
 */
export function setOnStopped(
  terminalId: string,
  cb: ((exitCode: number | null, signal: string | null) => void) | undefined
): void {
  const instance = instances.get(terminalId)
  if (instance) {
    instance.onStopped = cb
  }
  if (cb) {
    pendingOnStopped.set(terminalId, cb)
  } else {
    pendingOnStopped.delete(terminalId)
  }
}
