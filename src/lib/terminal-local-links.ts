import type { ILink, ILinkProvider, Terminal } from '@xterm/xterm'
import { invoke } from '@/lib/transport'
import type { AppPreferences } from '@/types/preferences'

export interface LocalTerminalLink {
  text: string
  index: number
  pathText: string
  line?: number
  column?: number
}

export interface LocalTerminalLinkRange {
  text: string
  range: {
    start: { x: number; y: number }
    end: { x: number; y: number }
  }
}

interface ResolvedLocalTerminalLink {
  path: string
  line?: number
  column?: number
}

type OpenResolvedLocalFile = (
  resolved: ResolvedLocalTerminalLink,
  editor: AppPreferences['editor'] | undefined
) => Promise<void>

const openResolvedLocalFile: OpenResolvedLocalFile = (resolved, editor) =>
  invoke('open_file_in_default_app', {
    path: resolved.path,
    editor,
    line: resolved.line,
    column: resolved.column,
  })

const LOCAL_LINK_REGEX =
  /(?:file:\/\/[^\s<>"'`]+|(?:\.{0,2}\/|[A-Za-z0-9_.-]+\/)[^\s<>"'`]+)/g
const TRAILING_PUNCTUATION = /[),.;!?\]}]+$/
const LINE_COLUMN_SUFFIX = /:(\d+)(?::(\d+))?$/

function trimCandidate(candidate: string): string {
  return candidate.replace(TRAILING_PUNCTUATION, '')
}

function decodeFileUrl(text: string): string | null {
  try {
    const url = new URL(text)
    if (url.protocol !== 'file:') return null
    return decodeURIComponent(url.pathname)
  } catch {
    return null
  }
}

function splitLineColumn(
  text: string
): Omit<LocalTerminalLink, 'text' | 'index'> {
  const lineMatch = LINE_COLUMN_SUFFIX.exec(text)
  if (!lineMatch) return { pathText: text }

  const withoutSuffix = text.slice(0, lineMatch.index)
  return {
    pathText: withoutSuffix,
    line: Number(lineMatch[1]),
    column: lineMatch[2] ? Number(lineMatch[2]) : undefined,
  }
}

function normalizeSlashes(path: string): string {
  return path.replace(/\/+/g, '/')
}

function normalizePath(path: string): string {
  const absolute = path.startsWith('/')
  const parts: string[] = []

  for (const rawPart of normalizeSlashes(path).split('/')) {
    if (!rawPart || rawPart === '.') continue
    if (rawPart === '..') {
      if (parts.length === 0) return absolute ? '/' : '..'
      parts.pop()
      continue
    }
    parts.push(rawPart)
  }

  return `${absolute ? '/' : ''}${parts.join('/')}` || (absolute ? '/' : '.')
}

function isUnderDirectory(path: string, directory: string): boolean {
  const normalizedPath = normalizePath(path)
  const normalizedDirectory = normalizePath(directory)
  return (
    normalizedPath === normalizedDirectory ||
    normalizedPath.startsWith(`${normalizedDirectory.replace(/\/$/, '')}/`)
  )
}

export function findLocalTerminalLinks(line: string): LocalTerminalLink[] {
  const links: LocalTerminalLink[] = []
  let match: RegExpExecArray | null

  LOCAL_LINK_REGEX.lastIndex = 0
  while ((match = LOCAL_LINK_REGEX.exec(line))) {
    const text = trimCandidate(match[0])
    if (!text || /^https?:\/\//i.test(text)) continue

    const decodedPath = text.startsWith('file://') ? decodeFileUrl(text) : null
    const split = splitLineColumn(decodedPath ?? text)
    if (!split.pathText.includes('/')) continue

    links.push({
      text,
      index: match.index,
      ...split,
    })
  }

  return links
}

export function resolveLocalTerminalLink(
  linkText: string,
  worktreePath: string
): ResolvedLocalTerminalLink | null {
  const text = trimCandidate(linkText)
  const decodedPath = text.startsWith('file://') ? decodeFileUrl(text) : null
  const split = splitLineColumn(decodedPath ?? text)
  if (split.pathText.startsWith('/')) {
    return {
      path: normalizePath(split.pathText),
      line: split.line,
      column: split.column,
    }
  }

  const path = normalizePath(`${worktreePath}/${split.pathText}`)
  if (!isUnderDirectory(path, worktreePath)) return null
  return { path, line: split.line, column: split.column }
}

interface TerminalRowText {
  text: string
  isWrapped: boolean
}

function mapStringIndexToRangePosition(
  rows: TerminalRowText[],
  firstY: number,
  stringIndex: number
): { x: number; y: number } {
  if (rows.length === 0) return { x: 1, y: firstY }

  let remaining = stringIndex

  for (let rowIndex = 0; rowIndex < rows.length; rowIndex += 1) {
    const row = rows[rowIndex]
    if (!row) continue
    const rowLength = row.text.length
    if (remaining <= rowLength) {
      return { x: remaining + 1, y: firstY + rowIndex }
    }
    remaining -= rowLength
  }

  const lastRow = rows[rows.length - 1]
  return { x: lastRow?.text.length ?? 1, y: firstY + rows.length - 1 }
}

export function computeLocalTerminalLinksForRows(
  rows: TerminalRowText[],
  firstY: number
): LocalTerminalLinkRange[] {
  const combinedText = rows.map(row => row.text).join('')

  return findLocalTerminalLinks(combinedText).map(link => ({
    text: link.text,
    range: {
      start: mapStringIndexToRangePosition(rows, firstY, link.index),
      end: mapStringIndexToRangePosition(
        rows,
        firstY,
        link.index + link.text.length - 1
      ),
    },
  }))
}

function getWrappedRows(
  terminal: Terminal,
  y: number
): { rows: TerminalRowText[]; firstY: number } {
  const buffer = terminal.buffer.active
  let firstIndex = y - 1
  let lastIndex = y - 1

  while (firstIndex > 0) {
    const currentLine = buffer.getLine(firstIndex)
    if (!currentLine?.isWrapped) break
    firstIndex -= 1
  }

  while (true) {
    const nextLine = buffer.getLine(lastIndex + 1)
    if (!nextLine?.isWrapped) break
    lastIndex += 1
  }

  const rows: TerminalRowText[] = []
  for (let index = firstIndex; index <= lastIndex; index += 1) {
    const line = buffer.getLine(index)
    if (!line) continue
    rows.push({
      text: line.translateToString(true),
      isWrapped: line.isWrapped,
    })
  }

  return { rows, firstY: firstIndex + 1 }
}

export class LocalTerminalLinkProvider implements ILinkProvider {
  constructor(
    private readonly terminal: Terminal,
    private readonly worktreePath: string,
    private readonly getEditor: () =>
      | AppPreferences['editor']
      | undefined = () => undefined,
    private readonly openFile: OpenResolvedLocalFile = openResolvedLocalFile
  ) {}

  provideLinks(
    y: number,
    callback: (links: ILink[] | undefined) => void
  ): void {
    const { rows, firstY } = getWrappedRows(this.terminal, y)
    if (rows.length === 0) {
      callback(undefined)
      return
    }

    const links = computeLocalTerminalLinksForRows(rows, firstY).map(link => ({
      text: link.text,
      range: link.range,
      activate: () => {
        const resolved = resolveLocalTerminalLink(link.text, this.worktreePath)
        if (!resolved) return
        void this.openFile(resolved, this.getEditor())
      },
    }))

    callback(links.length > 0 ? links : undefined)
  }
}
