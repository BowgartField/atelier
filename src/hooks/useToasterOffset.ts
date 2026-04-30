import { useBrowserStore } from '@/store/browser-store'
import { useChatStore } from '@/store/chat-store'
import { useUIStore } from '@/store/ui-store'
import { useProjectsStore } from '@/store/projects-store'
import { isNativeApp } from '@/lib/environment'

const BASE = 52
const GUTTER = 12

export function useToasterOffset() {
  const activeWorktreeId = useChatStore(s => s.activeWorktreeId)
  const sessionChatModalOpen = useUIStore(s => s.sessionChatModalOpen)
  const sessionChatModalWorktreeId = useUIStore(
    s => s.sessionChatModalWorktreeId
  )
  const selectedWorktreeId = useProjectsStore(s => s.selectedWorktreeId)

  // Match FloatingDock's resolution order so the hook tracks whichever
  // worktree is actually displaying browser surfaces.
  const currentWorktreeId = sessionChatModalOpen
    ? (sessionChatModalWorktreeId ?? activeWorktreeId ?? selectedWorktreeId)
    : (activeWorktreeId ?? selectedWorktreeId)

  const sidePaneOpen = useBrowserStore(s =>
    currentWorktreeId ? (s.sidePaneOpen[currentWorktreeId] ?? false) : false
  )
  const bottomPanelOpen = useBrowserStore(s =>
    currentWorktreeId ? (s.bottomPanelOpen[currentWorktreeId] ?? false) : false
  )
  const modalOpen = useBrowserStore(s =>
    currentWorktreeId ? (s.modalOpen[currentWorktreeId] ?? false) : false
  )
  const sidePaneWidth = useBrowserStore(s => s.sidePaneWidth)
  const bottomPanelHeight = useBrowserStore(s => s.bottomPanelHeight)
  const modalDockMode = useBrowserStore(s => s.modalDockMode)
  const modalWidth = useBrowserStore(s => s.modalWidth)
  const modalHeight = useBrowserStore(s => s.modalHeight)

  if (!isNativeApp()) return `${BASE}px`

  let right = BASE
  let bottom = BASE

  if (sidePaneOpen) right += sidePaneWidth + GUTTER
  if (bottomPanelOpen) bottom += bottomPanelHeight + GUTTER

  if (modalOpen) {
    if (modalDockMode === 'right' || modalDockMode === 'floating') {
      right += modalWidth + GUTTER
    } else if (modalDockMode === 'bottom') {
      bottom += modalHeight + GUTTER
    }
  }

  return { top: BASE, right, bottom, left: BASE }
}
