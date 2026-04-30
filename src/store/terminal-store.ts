import { create } from 'zustand'
import { getFilename } from '@/lib/path-utils'
import { generateId } from '@/lib/uuid'
import type { ModalTerminalDockMode } from '@/types/ui-state'
import { useBrowserStore } from './browser-store'

/** A single terminal instance */
export interface TerminalInstance {
  id: string
  worktreeId: string
  command: string | null
  label: string
}

interface TerminalState {
  // Terminal instances per worktree (worktreeId -> terminals)
  terminals: Record<string, TerminalInstance[]>
  // Active terminal ID per worktree
  activeTerminalIds: Record<string, string>
  // Set of running terminal IDs (have active PTY process)
  runningTerminals: Set<string>
  // Set of terminal IDs that exited with non-zero exit code (crash/failure)
  failedTerminals: Set<string>
  // Whether terminal panel is expanded (false = collapsed/minimized) - global since only one worktree visible
  terminalVisible: boolean
  // Whether terminal panel is open per worktree (worktreeId -> open)
  terminalPanelOpen: Record<string, boolean>
  terminalHeight: number

  // Modal terminal drawer state
  modalTerminalOpen: Record<string, boolean>
  modalTerminalDockMode: ModalTerminalDockMode
  modalTerminalWidth: number
  modalTerminalHeight: number

  setTerminalVisible: (visible: boolean) => void
  setTerminalPanelOpen: (worktreeId: string, open: boolean) => void
  isTerminalPanelOpen: (worktreeId: string) => boolean
  toggleTerminal: (worktreeId: string) => void
  setTerminalHeight: (height: number) => void

  // Modal terminal drawer methods
  setModalTerminalOpen: (worktreeId: string, open: boolean) => void
  toggleModalTerminal: (worktreeId: string) => void
  setModalTerminalDockMode: (dockMode: ModalTerminalDockMode) => void
  setModalTerminalWidth: (width: number) => void
  setModalTerminalHeight: (height: number) => void

  // Terminal instance management
  addTerminal: (
    worktreeId: string,
    command?: string | null,
    label?: string
  ) => string
  removeTerminal: (worktreeId: string, terminalId: string) => void
  setActiveTerminal: (worktreeId: string, terminalId: string) => void
  getTerminals: (worktreeId: string) => TerminalInstance[]
  getActiveTerminal: (worktreeId: string) => TerminalInstance | null

  // Running state (terminal has active PTY)
  setTerminalRunning: (terminalId: string, running: boolean) => void
  isTerminalRunning: (terminalId: string) => boolean

  // Failed state (terminal exited with non-zero code)
  setTerminalFailed: (terminalId: string, failed: boolean) => void
  isTerminalFailed: (terminalId: string) => boolean

  // Start a run command (creates new terminal with command)
  startRun: (worktreeId: string, command: string) => string

  // Close all terminals for a worktree (returns terminal IDs that need to be stopped)
  closeAllTerminals: (worktreeId: string) => string[]
}

function generateTerminalId(): string {
  return generateId()
}

/** Close every browser surface for this worktree — terminal modal and
 * browser surfaces are mutually exclusive. Called inside terminal-store
 * actions when opening the terminal modal. */
function closeBrowserSurfacesFor(worktreeId: string): void {
  const browser = useBrowserStore.getState()
  const sideOpen = browser.sidePaneOpen[worktreeId] ?? false
  const modalOpen = browser.modalOpen[worktreeId] ?? false
  const bottomOpen = browser.bottomPanelOpen[worktreeId] ?? false
  if (!sideOpen && !modalOpen && !bottomOpen) return
  useBrowserStore.setState({
    sidePaneOpen: sideOpen
      ? { ...browser.sidePaneOpen, [worktreeId]: false }
      : browser.sidePaneOpen,
    modalOpen: modalOpen
      ? { ...browser.modalOpen, [worktreeId]: false }
      : browser.modalOpen,
    bottomPanelOpen: bottomOpen
      ? { ...browser.bottomPanelOpen, [worktreeId]: false }
      : browser.bottomPanelOpen,
  })
}

function getDefaultLabel(command: string | null): string {
  if (!command) return 'Shell'
  // Extract first word or command name
  const firstWord = command.split(' ')[0] ?? command
  // Remove path if present (cross-platform)
  const name = getFilename(firstWord)
  return name.length > 20 ? name.slice(0, 17) + '...' : name
}

export const useTerminalStore = create<TerminalState>((set, get) => ({
  terminals: {},
  activeTerminalIds: {},
  runningTerminals: new Set(),
  failedTerminals: new Set(),
  terminalVisible: false,
  terminalPanelOpen: {},
  terminalHeight: 30,
  modalTerminalOpen: {},
  modalTerminalDockMode: 'floating',
  modalTerminalWidth: 400,
  modalTerminalHeight: 280,

  setTerminalVisible: visible =>
    set(state =>
      state.terminalVisible === visible ? state : { terminalVisible: visible }
    ),

  setTerminalPanelOpen: (worktreeId, open) =>
    set(state => {
      if ((state.terminalPanelOpen[worktreeId] ?? false) === open) return state
      return {
        terminalPanelOpen: {
          ...state.terminalPanelOpen,
          [worktreeId]: open,
        },
      }
    }),

  isTerminalPanelOpen: worktreeId =>
    get().terminalPanelOpen[worktreeId] ?? false,

  toggleTerminal: worktreeId =>
    set(state => ({
      terminalVisible: !state.terminalVisible,
      // Also open the panel for this worktree if making visible
      terminalPanelOpen: !state.terminalVisible
        ? { ...state.terminalPanelOpen, [worktreeId]: true }
        : state.terminalPanelOpen,
    })),

  setTerminalHeight: height =>
    set(state =>
      state.terminalHeight === height ? state : { terminalHeight: height }
    ),

  setModalTerminalOpen: (worktreeId, open) => {
    const current =
      useTerminalStore.getState().modalTerminalOpen[worktreeId] ?? false
    if (current === open) return
    if (open) closeBrowserSurfacesFor(worktreeId)
    set(state => ({
      modalTerminalOpen: { ...state.modalTerminalOpen, [worktreeId]: open },
    }))
  },

  toggleModalTerminal: worktreeId => {
    const current =
      useTerminalStore.getState().modalTerminalOpen[worktreeId] ?? false
    const next = !current
    if (next) closeBrowserSurfacesFor(worktreeId)
    set(state => ({
      modalTerminalOpen: {
        ...state.modalTerminalOpen,
        [worktreeId]: next,
      },
    }))
  },

  setModalTerminalDockMode: dockMode =>
    set(state =>
      state.modalTerminalDockMode === dockMode
        ? state
        : { modalTerminalDockMode: dockMode }
    ),

  setModalTerminalWidth: width =>
    set(state =>
      state.modalTerminalWidth === width ? state : { modalTerminalWidth: width }
    ),

  setModalTerminalHeight: height =>
    set(state =>
      state.modalTerminalHeight === height
        ? state
        : { modalTerminalHeight: height }
    ),

  addTerminal: (worktreeId, command = null, label) => {
    const id = generateTerminalId()
    const terminal: TerminalInstance = {
      id,
      worktreeId,
      command,
      label: label ?? getDefaultLabel(command),
    }

    set(state => {
      const existing = state.terminals[worktreeId] ?? []
      return {
        terminals: {
          ...state.terminals,
          [worktreeId]: [...existing, terminal],
        },
        activeTerminalIds: {
          ...state.activeTerminalIds,
          [worktreeId]: id,
        },
        terminalPanelOpen: {
          ...state.terminalPanelOpen,
          [worktreeId]: true,
        },
        terminalVisible: true,
      }
    })

    return id
  },

  removeTerminal: (worktreeId, terminalId) =>
    set(state => {
      const existing = state.terminals[worktreeId] ?? []
      const filtered = existing.filter(t => t.id !== terminalId)

      // Update running terminals
      const newRunning = new Set(state.runningTerminals)
      newRunning.delete(terminalId)

      // Update failed terminals
      const newFailed = new Set(state.failedTerminals)
      newFailed.delete(terminalId)

      // Update active terminal if needed
      const currentActiveId = state.activeTerminalIds[worktreeId] ?? ''
      const newActiveId =
        currentActiveId === terminalId
          ? (filtered[filtered.length - 1]?.id ?? '')
          : currentActiveId

      return {
        terminals: {
          ...state.terminals,
          [worktreeId]: filtered,
        },
        activeTerminalIds: {
          ...state.activeTerminalIds,
          [worktreeId]: newActiveId,
        },
        runningTerminals: newRunning,
        failedTerminals: newFailed,
      }
    }),

  setActiveTerminal: (worktreeId, terminalId) =>
    set(state => ({
      activeTerminalIds: {
        ...state.activeTerminalIds,
        [worktreeId]: terminalId,
      },
    })),

  getTerminals: worktreeId => get().terminals[worktreeId] ?? [],

  getActiveTerminal: worktreeId => {
    const terminals = get().terminals[worktreeId] ?? []
    const activeId = get().activeTerminalIds[worktreeId]
    return terminals.find(t => t.id === activeId) ?? null
  },

  setTerminalRunning: (terminalId, running) =>
    set(state => {
      if (running === state.runningTerminals.has(terminalId)) return state
      const newSet = new Set(state.runningTerminals)
      if (running) {
        newSet.add(terminalId)
      } else {
        newSet.delete(terminalId)
      }
      return { runningTerminals: newSet }
    }),

  isTerminalRunning: terminalId => get().runningTerminals.has(terminalId),

  setTerminalFailed: (terminalId, failed) =>
    set(state => {
      if (failed === state.failedTerminals.has(terminalId)) return state
      const newSet = new Set(state.failedTerminals)
      if (failed) {
        newSet.add(terminalId)
      } else {
        newSet.delete(terminalId)
      }
      return { failedTerminals: newSet }
    }),

  isTerminalFailed: terminalId => get().failedTerminals.has(terminalId),

  startRun: (worktreeId, command) => {
    const state = get()
    const terminals = state.terminals[worktreeId] ?? []

    // Check if there's already a running terminal with this command
    const existingTerminal = terminals.find(
      t => t.command === command && state.runningTerminals.has(t.id)
    )

    if (existingTerminal) {
      // Focus the existing terminal instead of creating a new one
      set({
        activeTerminalIds: {
          ...state.activeTerminalIds,
          [worktreeId]: existingTerminal.id,
        },
        terminalVisible: true,
        terminalPanelOpen: {
          ...state.terminalPanelOpen,
          [worktreeId]: true,
        },
      })
      return existingTerminal.id
    }

    // Clear stale failed IDs for this worktree's command terminals
    const failedIds = terminals.filter(
      t => t.command && state.failedTerminals.has(t.id)
    )
    if (failedIds.length > 0) {
      const newFailed = new Set(state.failedTerminals)
      for (const t of failedIds) newFailed.delete(t.id)
      set({ failedTerminals: newFailed })
    }

    // No existing running terminal, create a new one (addTerminal sets terminalPanelOpen)
    return get().addTerminal(worktreeId, command)
  },

  closeAllTerminals: worktreeId => {
    const state = get()
    const terminals = state.terminals[worktreeId] ?? []
    const terminalIds = terminals.map(t => t.id)

    // Remove all running/failed terminal IDs for this worktree
    const newRunning = new Set(state.runningTerminals)
    const newFailed = new Set(state.failedTerminals)
    for (const id of terminalIds) {
      newRunning.delete(id)
      newFailed.delete(id)
    }

    set({
      terminals: {
        ...state.terminals,
        [worktreeId]: [],
      },
      activeTerminalIds: {
        ...state.activeTerminalIds,
        [worktreeId]: '',
      },
      runningTerminals: newRunning,
      failedTerminals: newFailed,
      terminalPanelOpen: {
        ...state.terminalPanelOpen,
        [worktreeId]: false,
      },
      // Don't set terminalVisible=false as that's global and affects other worktrees
    })

    return terminalIds
  },
}))
