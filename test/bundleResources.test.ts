import { existsSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

const root = join(__dirname, '..')

describe('the files the bundle ships beside the app', () => {
  it('are all there, the font licence among them', () => {
    const config = JSON.parse(readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8'))
    const resources = Object.keys(config.bundle.resources as Record<string, string>)
    expect(resources).toContain('../licenses/Inter-OFL-1.1.txt')
    for (const from of resources) expect(existsSync(join(root, 'src-tauri', from)), from).toBe(true)
    expect(readFileSync(join(root, 'licenses/Inter-OFL-1.1.txt'), 'utf8')).toContain('SIL Open Font License')
  })
})
