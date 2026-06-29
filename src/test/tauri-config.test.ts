import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

function readMainWindowConfig(configPath: string) {
  const rawConfig = readFileSync(join(process.cwd(), configPath), 'utf8')
  const config = JSON.parse(rawConfig) as {
    app?: { windows?: { dragDropEnabled?: boolean }[] }
  }

  return config.app?.windows?.[0]
}

describe('Tauri drag/drop configuration', () => {
  it.each(['src-tauri/tauri.conf.json', 'src-tauri/tauri.conf.dev.json'])(
    'disables native webview drag/drop in %s so browser DnD can handle reordering',
    configPath => {
      expect(readMainWindowConfig(configPath)?.dragDropEnabled).toBe(false)
    }
  )
})
