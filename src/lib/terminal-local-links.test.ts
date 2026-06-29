import type { ILink } from '@xterm/xterm'
import { describe, expect, it } from 'vitest'
import {
  computeLocalTerminalLinksForRows,
  findLocalTerminalLinks,
  LocalTerminalLinkProvider,
  resolveLocalTerminalLink,
} from './terminal-local-links'

describe('terminal local links', () => {
  it('detects absolute file paths with line and column suffixes', () => {
    const links = findLocalTerminalLinks(
      'see /Users/me/app/src/main.ts:12:3 now'
    )
    expect(links).toEqual([
      {
        text: '/Users/me/app/src/main.ts:12:3',
        index: 4,
        pathText: '/Users/me/app/src/main.ts',
        line: 12,
        column: 3,
      },
    ])
  })

  it('detects file URLs and decodes the filesystem path', () => {
    const links = findLocalTerminalLinks(
      'open file:///Users/me/My%20App/main.ts:8'
    )
    expect(links[0]).toMatchObject({
      text: 'file:///Users/me/My%20App/main.ts:8',
      pathText: '/Users/me/My App/main.ts',
      line: 8,
    })
  })

  it('resolves relative links inside the worktree', () => {
    expect(resolveLocalTerminalLink('src/main.ts:5', '/Users/me/app')).toEqual({
      path: '/Users/me/app/src/main.ts',
      line: 5,
      column: undefined,
    })
  })

  it('allows absolute local links outside the worktree', () => {
    expect(
      resolveLocalTerminalLink('/tmp/output.log:9', '/Users/me/app')
    ).toEqual({
      path: '/tmp/output.log',
      line: 9,
      column: undefined,
    })
  })

  it('rejects relative links that escape the worktree', () => {
    expect(
      resolveLocalTerminalLink('../secret.txt', '/Users/me/app')
    ).toBeNull()
  })

  it('maps a local path across wrapped terminal rows', () => {
    const rows = [
      {
        text: '    /Users/me/app/src-tauri/target/release-fast/bundle/dmg/Jean_0.1.52_aarc',
        isWrapped: false,
      },
      { text: 'h64.dmg', isWrapped: true },
    ]

    const links = computeLocalTerminalLinksForRows(rows, 1)

    expect(links).toEqual([
      {
        text: '/Users/me/app/src-tauri/target/release-fast/bundle/dmg/Jean_0.1.52_aarch64.dmg',
        range: {
          start: { x: 5, y: 1 },
          end: { x: 7, y: 2 },
        },
      },
    ])
  })

  it('opens the resolved filesystem path with editor preference and location', () => {
    const opened: {
      path: string
      editor: string | undefined
      line?: number
      column?: number
    }[] = []
    const provider = new LocalTerminalLinkProvider(
      {
        buffer: {
          active: {
            getLine: (index: number) =>
              index === 0
                ? {
                    isWrapped: false,
                    translateToString: () => 'src/main.ts:12:3',
                  }
                : undefined,
          },
        },
      } as never,
      '/Users/me/app',
      () => 'cursor',
      (resolved, editor) => {
        opened.push({ ...resolved, editor })
        return Promise.resolve()
      }
    )

    let providedLinks: ILink[] | undefined
    provider.provideLinks(1, links => {
      providedLinks = links
    })
    providedLinks?.[0]?.activate(undefined as never, '')

    expect(opened).toEqual([
      {
        path: '/Users/me/app/src/main.ts',
        editor: 'cursor',
        line: 12,
        column: 3,
      },
    ])
  })
})
