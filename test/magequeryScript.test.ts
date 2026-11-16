import { chmodSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  ASSETS,
  SOURCE,
  Untrusted,
  VERSION,
  binaryName,
  checkEntries,
  hostTriple,
  plan,
  sha256,
  stamp,
  tarCommand,
  upToDate,
  verify
} from '../scripts/magequery.mjs'

/**
 * The fetch script for the bundled magequery, without the network: the hash
 * check, the names Tauri looks for, and which target gets what.
 */
describe('scripts/magequery.mjs', () => {
  it('pins the same version the Rust side says it was written against', () => {
    const rust = readFileSync(join(__dirname, '../src-tauri/src/frameworks/magequery.rs'), 'utf8')
    const pinned = rust.match(/pub const PINNED: &str = "([^"]+)";/)?.[1]
    expect(pinned).toBe(VERSION)
  })

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

  it('marks a mismatch as one --optional does not pass over', () => {
    const bytes = Buffer.from('magequery fixture')
    let thrown: unknown
    try {
      verify(bytes, sha256(Buffer.from('tampered')), 'fixture')
    } catch (error) {
      thrown = error
    }
    expect(thrown).toBeInstanceOf(Untrusted)
  })

  it('refuses an archive whose entries leave the folder it is unpacked into', () => {
    expect(checkEntries(['magequery-aarch64-apple-darwin/', 'magequery-aarch64-apple-darwin/magequery', './LICENSE'])).toBe(true)
    for (const name of ['../evil', 'a/../../evil', '/etc/passwd', 'C:\\Windows\\x', 'a\\..\\..\\x', '\\\\host\\x']) {
      expect(() => checkEntries([name], 'x.tar.gz'), name).toThrow(Untrusted)
    }
    // Two dots inside a name are a name.
    expect(checkEntries(['a/b..c/d', 'notes..txt'])).toBe(true)
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

  it.skipIf(process.platform === 'win32')('gives an up-to-date binary its execute bit back', () => {
    const dir = mkdtempSync(join(tmpdir(), 'magequery-mode-'))
    try {
      const name = binaryName('x86_64-unknown-linux-gnu')
      const file = join(dir, name)
      writeFileSync(file, 'binary')
      chmodSync(file, 0o644)
      stamp(dir, name)
      expect(upToDate(dir, name)).toBe(true)
      expect(statSync(file).mode & 0o111).not.toBe(0)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('skips a binary that is already this version, and not an empty placeholder or a changed file', () => {
    const dir = mkdtempSync(join(tmpdir(), 'magequery-stamp-'))
    try {
      const name = binaryName('aarch64-apple-darwin')
      const file = join(dir, name)
      const stampFile = join(dir, '.magequery-version.json')
      writeFileSync(file, '')
      writeFileSync(stampFile, JSON.stringify({ [name]: { version: VERSION, sha256: sha256(Buffer.from('')) } }))
      expect(upToDate(dir, name)).toBe(false)
      writeFileSync(file, 'binary')
      stamp(dir, name)
      expect(upToDate(dir, name)).toBe(true)
      // Changed after it was stamped: not the file that was fetched.
      writeFileSync(file, 'binarY')
      expect(upToDate(dir, name)).toBe(false)
      writeFileSync(file, 'binary')
      expect(upToDate(dir, name)).toBe(true)
      // Another version, or a stamp from before hashes were kept.
      writeFileSync(stampFile, JSON.stringify({ [name]: { version: '0.0.1', sha256: sha256(Buffer.from('binary')) } }))
      expect(upToDate(dir, name)).toBe(false)
      writeFileSync(stampFile, JSON.stringify({ [name]: VERSION }))
      expect(upToDate(dir, name)).toBe(false)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})
