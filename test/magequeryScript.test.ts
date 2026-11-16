import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  ASSETS,
  SOURCE,
  VERSION,
  binaryName,
  hostTriple,
  plan,
  sha256,
  tarCommand,
  upToDate,
  verify
} from '../scripts/magequery.mjs'

/**
 * The fetch script for the bundled magequery, without the network: the hash
 * check, the names Tauri looks for, and which target gets what.
 */
describe('scripts/magequery.mjs', () => {
  it('accepts a download whose sha256 is the pinned one', () => {
    const bytes = Buffer.from('magequery fixture')
    expect(verify(bytes, sha256(bytes), 'fixture')).toBe(true)
    expect(verify(bytes, sha256(bytes).toUpperCase(), 'fixture')).toBe(true)
  })

  it('refuses a download whose sha256 is not', () => {
    const bytes = Buffer.from('magequery fixture')
    const other = sha256(Buffer.from('something else'))
    expect(() => verify(bytes, other, 'magequery-x.tar.gz')).toThrow(/does not match its pinned sha256/)
  })

  it('hashes the way sha256sum does', () => {
    expect(sha256(Buffer.from('abc'))).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
  })

  it('pins a hash for every asset and the source', () => {
    for (const asset of [...Object.values(ASSETS), SOURCE]) {
      expect(asset.sha256).toMatch(/^[0-9a-f]{64}$/)
    }
    expect(VERSION).toMatch(/^\d+\.\d+\.\d+$/)
  })

  it('names the binary the way Tauri looks for a sidecar', () => {
    expect(binaryName('aarch64-apple-darwin')).toBe('magequery-aarch64-apple-darwin')
    expect(binaryName('x86_64-pc-windows-msvc')).toBe('magequery-x86_64-pc-windows-msvc.exe')
    expect(binaryName('universal-apple-darwin')).toBe('magequery-universal-apple-darwin')
  })

  it('works out the target of the machine it runs on', () => {
    expect(hostTriple('darwin', 'arm64')).toBe('aarch64-apple-darwin')
    expect(hostTriple('darwin', 'x64')).toBe('x86_64-apple-darwin')
    expect(hostTriple('linux', 'x64')).toBe('x86_64-unknown-linux-gnu')
    expect(hostTriple('win32', 'x64')).toBe('x86_64-pc-windows-msvc')
    expect(() => hostTriple('win32', 'arm64')).toThrow()
    expect(() => hostTriple('linux', 'ia32')).toThrow()
  })

  it('downloads, builds from source or joins, by target', () => {
    expect(plan('aarch64-apple-darwin')).toMatchObject({ kind: 'download' })
    // No Intel Mac release: built from the pinned source.
    expect(plan('x86_64-apple-darwin')).toEqual({ kind: 'build' })
    expect(plan('universal-apple-darwin')).toEqual({ kind: 'universal' })
    // The static build for x86_64 Linux, so the AppImage keeps its glibc floor.
    expect(plan('x86_64-unknown-linux-gnu')).toMatchObject({
      asset: { file: 'magequery-x86_64-unknown-linux-musl.tar.gz' }
    })
    expect(() => plan('riscv64gc-unknown-linux-gnu')).toThrow(/No magequery/)
  })

  it('uses Windows’ own tar, which reads zip files', () => {
    expect(tarCommand('linux')).toBe('tar')
    expect(tarCommand('win32', 'C:\\Windows')).toMatch(/System32.tar\.exe$/)
  })

  it('skips a binary that is already this version, and not an empty placeholder', () => {
    const dir = mkdtempSync(join(tmpdir(), 'magequery-stamp-'))
    try {
      const name = binaryName('aarch64-apple-darwin')
      writeFileSync(join(dir, name), '')
      writeFileSync(join(dir, '.magequery-version.json'), JSON.stringify({ [name]: VERSION }))
      expect(upToDate(dir, name)).toBe(false)
      writeFileSync(join(dir, name), 'binary')
      expect(upToDate(dir, name)).toBe(true)
      writeFileSync(join(dir, '.magequery-version.json'), JSON.stringify({ [name]: '0.0.1' }))
      expect(upToDate(dir, name)).toBe(false)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
