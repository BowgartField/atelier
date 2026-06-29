interface WorktreeLabelContextMenuOptions {
  isMobile: boolean
  isNative: boolean
}

interface WorktreeTextSelectionOptions {
  isMobile: boolean
}

export function shouldShowWorktreeLabelContextMenu({
  isMobile,
  isNative,
}: WorktreeLabelContextMenuOptions) {
  return isMobile || isNative
}

export function shouldDisableWorktreeTextSelection({
  isMobile,
}: WorktreeTextSelectionOptions) {
  return isMobile
}
