#!/usr/bin/env node
/**
 * Fetches the magequery binary gitnoob bundles as a sidecar.
 *
 *   node scripts/magequery.mjs                          # this machine
 *   node scripts/magequery.mjs --target universal-apple-darwin
 *   node scripts/magequery.mjs --optional               # warn instead of failing (npm run app)
 *
 * magequery (https://github.com/cresset-tools/magequery, EUPL-1.2) reads a
 * Magento installation's config. gitnoob ships the unmodified binary next to
 * its own executable (tauri.conf.json `bundle.externalBin`); Tauri wants it at
 * src-tauri/binaries/magequery-<target triple>[.exe] at build time.
 *
 * Every download is checked against the sha256 written below and refused on a
 * mismatch, before anything is unpacked; a mismatch fails the run even with
 * --optional. To move to a new version: change VERSION, then copy the hashes
 * from that release's sha256.sum and source.tar.gz.sha256 into ASSETS and
 * SOURCE, and PINNED in src-tauri/src/frameworks/magequery.rs.
 *
 * There is no Intel Mac build in the releases, so for x86_64-apple-darwin it is
 * built from the pinned source with cargo, and for the universal app the two
 * Mac builds are joined with lipo. A machine that cannot build for Intel fails
 * here rather than shipping half a universal binary.
 *
 * Plain Node, no dependencies. Running it again does nothing when the binary
 * of this version is already there (a stamp file next to it says which).
 */
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync, copyFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const VERSION = '0.24.1'
export const REPOSITORY = 'https://github.com/cresset-tools/magequery'
export const BASE = `${REPOSITORY}/releases/download/magequery-v${VERSION}`

/**
 * The release asset for each target triple gitnoob builds for. x86_64 Linux
 * takes the static musl build: the gnu one carries the glibc of whatever it
 * was built on as a floor, and the AppImage is built on 22.04 on purpose.
 */
export const ASSETS = {
  'aarch64-apple-darwin': {
    file: 'magequery-aarch64-apple-darwin.tar.gz',
    sha256: '8d8315a72e42bc7fd0c9c53168fb285d1ca1a8abcce5cb23c37c9f897974c01b'
  },
  'aarch64-unknown-linux-gnu': {
    file: 'magequery-aarch64-unknown-linux-gnu.tar.gz',
    sha256: 'd90cca776729f0f3112346f5e1e275645f98907fdab0aea03086dc30326b464c'
  },
  'x86_64-unknown-linux-gnu': {
    file: 'magequery-x86_64-unknown-linux-musl.tar.gz',
    sha256: 'fd258fb6ff1f7eb088aa0cc5e3d13e8f3027e5b7e339dd4e505b6b061a39039b'
  },
  'x86_64-unknown-linux-musl': {
    file: 'magequery-x86_64-unknown-linux-musl.tar.gz',
    sha256: 'fd258fb6ff1f7eb088aa0cc5e3d13e8f3027e5b7e339dd4e505b6b061a39039b'
  },
  'x86_64-pc-windows-msvc': {
    file: 'magequery-x86_64-pc-windows-msvc.zip',
    sha256: '35fa4157840d3251bbe117a75cadaee20c1b3fa81616816cdc9d35dd32be6691'
  }
}

/** The source of the same release, for the Intel Mac build. */
export const SOURCE = {
  file: 'source.tar.gz',
  sha256: 'ef1643eebbf24ce4dcddae464f768c722487e058cb1e61f3be618546605202cd',
  folder: `magequery-${VERSION}`
}

const here = dirname(fileURLToPath(import.meta.url))
export const OUT = join(here, '..', 'src-tauri', 'binaries')
const STAMP = '.magequery-version.json'

/** What Tauri looks for: `magequery-<triple>`, with `.exe` on Windows. */
export function binaryName(triple) {
  return `magequery-${triple}${triple.includes('windows') ? '.exe' : ''}`
}

/** The target triple of the machine this runs on. */
export function hostTriple(platform = process.platform, arch = process.arch) {
  const cpu = { arm64: 'aarch64', x64: 'x86_64' }[arch]
  if (!cpu) throw new Error(`No magequery build for the ${arch} processor`)
  if (platform === 'darwin') return `${cpu}-apple-darwin`
  if (platform === 'linux') return `${cpu}-unknown-linux-gnu`
  if (platform === 'win32' && cpu === 'x86_64') return 'x86_64-pc-windows-msvc'
  throw new Error(`No magequery build for ${platform} on ${arch}`)
}

export function sha256(buffer) {
  return createHash('sha256').update(buffer).digest('hex')
}

/**
 * A download that is not what was pinned, or an archive that is not shaped
 * like one. Never passed over, `--optional` or not: being offline is a
 * reason to go on without magequery, a file that is not the pinned one is not.
 */
export class Untrusted extends Error {}

/** Refuses a download whose hash is not the pinned one. */
export function verify(buffer, expected, name) {
  const actual = sha256(buffer)
  if (actual !== expected.toLowerCase()) {
    throw new Untrusted(`${name} does not match its pinned sha256 (expected ${expected}, got ${actual}). Not using it.`)
  }
  return true
}

/**
 * Refuses an archive with an entry that would land outside the folder it is
 * unpacked into: an absolute path, a drive letter, or a `..` on the way.
 * The archives are hash-pinned, so this only ever fires on one that was
 * pinned wrongly; it is here so that unpacking never depends on which tar
 * a machine has and what that one does with such names.
 */
export function checkEntries(names, archive = 'The archive') {
  for (const name of names) {
    const parts = name.split(/[\\/]/)
    if (name.startsWith('/') || name.startsWith('\\') || /^[A-Za-z]:/.test(name) || parts.includes('..')) {
      throw new Untrusted(`${archive} has an entry that leaves its folder (${name}). Not unpacking it.`)
    }
  }
  return true
}

/**
 * How a target is made: one release asset, a build from source (Intel Mac),
 * or both Mac builds joined (universal).
 */
export function plan(triple) {
  if (triple === 'universal-apple-darwin') return { kind: 'universal' }
  if (triple === 'x86_64-apple-darwin') return { kind: 'build' }
  const asset = ASSETS[triple]
  if (!asset) throw new Error(`No magequery ${VERSION} build for ${triple}`)
  return { kind: 'download', asset }
}

/** Whether `name` in `dir` is already this version. */
export function upToDate(dir, name) {
  const file = join(dir, name)
  if (!existsSync(file) || statSync(file).size === 0) return false
  try {
    const stamp = JSON.parse(readFileSync(join(dir, STAMP), 'utf8'))
    return stamp[name] === VERSION
  } catch {
    return false
  }
}

function stamp(dir, name) {
  let all = {}
  try {
    all = JSON.parse(readFileSync(join(dir, STAMP), 'utf8'))
  } catch {}
  all[name] = VERSION
  writeFileSync(join(dir, STAMP), `${JSON.stringify(all, null, 2)}\n`)
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options })
  if (result.error) throw new Error(`Could not run ${command}: ${result.error.message}`)
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed with status ${result.status}`)
}

async function download(file, expected) {
  const url = `${BASE}/${file}`
  console.log(`magequery: downloading ${url}`)
  // Ten minutes at most: a stalled connection fails the step, not hangs it.
  const response = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(600_000) })
  if (!response.ok) throw new Error(`Could not download ${url}: ${response.status} ${response.statusText}`)
  const buffer = Buffer.from(await response.arrayBuffer())
  verify(buffer, expected, file)
  return buffer
}

/** The first file called `name` under `dir`. */
function findFile(dir, name) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name)
    if (entry.isFile() && entry.name === name) return path
    if (entry.isDirectory()) {
      const found = findFile(path, name)
      if (found) return found
    }
  }
  return null
}

/**
 * Windows' own bsdtar, which reads zip files and drive letters. In a Git Bash
 * step the PATH finds GNU tar first, which does neither.
 */
export function tarCommand(platform = process.platform, root = process.env.SystemRoot) {
  return platform === 'win32' ? join(root ?? 'C:\\Windows', 'System32', 'tar.exe') : 'tar'
}

/** Unpacks an archive (tar.gz, or zip with Windows' and macOS's bsdtar). */
function unpack(buffer, file, work) {
  const archive = join(work, file)
  writeFileSync(archive, buffer)
  const into = join(work, 'unpacked')
  mkdirSync(into, { recursive: true })
  const listing = spawnSync(tarCommand(), ['-tf', archive], { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 })
  if (listing.error || listing.status !== 0) {
    throw new Error(`Could not list ${file}: ${listing.error?.message ?? listing.stderr?.trim() ?? listing.status}`)
  }
  checkEntries(listing.stdout.split(/\r?\n/).filter(Boolean), file)
  run(tarCommand(), ['-xf', archive, '-C', into])
  return into
}

function install(from, dir, name) {
  mkdirSync(dir, { recursive: true })
  const to = join(dir, name)
  copyFileSync(from, to)
  chmodSync(to, 0o755)
  stamp(dir, name)
  console.log(`magequery: ${to} (${(statSync(to).size / 1048576).toFixed(1)} MB)`)
  return to
}

async function fetchRelease(triple, dir, work) {
  const { asset } = plan(triple)
  const buffer = await download(asset.file, asset.sha256)
  const into = unpack(buffer, asset.file, work)
  const binary = findFile(into, triple.includes('windows') ? 'magequery.exe' : 'magequery')
  if (!binary) throw new Error(`${asset.file} has no magequery binary in it`)
  return install(binary, dir, binaryName(triple))
}

/** Builds the Intel Mac binary from the pinned source. */
async function buildIntel(dir, work) {
  const triple = 'x86_64-apple-darwin'
  const buffer = await download(SOURCE.file, SOURCE.sha256)
  const into = unpack(buffer, SOURCE.file, work)
  const source = join(into, SOURCE.folder)
  console.log('magequery: building for x86_64-apple-darwin from source')
  try {
    run('cargo', ['build', '--release', '--locked', '--target', triple, '--bin', 'magequery'], { cwd: source })
  } catch (error) {
    throw new Error(
      `Could not build magequery for ${triple}. Install the target with ` +
        `\`rustup target add ${triple}\` and try again. (${error.message})`
    )
  }
  const binary = join(source, 'target', triple, 'release', 'magequery')
  if (!existsSync(binary)) throw new Error(`The ${triple} build left no binary at ${binary}`)
  return install(binary, dir, binaryName(triple))
}

export async function fetchMagequery(triple, dir = OUT) {
  const work = mkdtempSync(join(tmpdir(), 'gitnoob-magequery-'))
  try {
    const step = plan(triple)
    if (step.kind === 'download') {
      if (!upToDate(dir, binaryName(triple))) await fetchRelease(triple, dir, work)
      return
    }
    if (step.kind === 'build') {
      if (!upToDate(dir, binaryName(triple))) await buildIntel(dir, work)
      return
    }
    // Universal: each architecture's build needs its own file, and the
    // bundler takes the joined one.
    const arm = 'aarch64-apple-darwin'
    const intel = 'x86_64-apple-darwin'
    const universal = binaryName('universal-apple-darwin')
    if (!upToDate(dir, binaryName(arm))) {
      mkdirSync(join(work, 'arm'))
      await fetchRelease(arm, dir, join(work, 'arm'))
    }
    if (!upToDate(dir, binaryName(intel))) {
      mkdirSync(join(work, 'intel'))
      await buildIntel(dir, join(work, 'intel'))
    }
    if (!upToDate(dir, universal)) {
      const joined = join(work, 'magequery-universal')
      run('lipo', ['-create', '-output', joined, join(dir, binaryName(arm)), join(dir, binaryName(intel))])
      const check = spawnSync('lipo', ['-archs', joined], { encoding: 'utf8' })
      const archs = (check.stdout ?? '').trim().split(/\s+/)
      if (!archs.includes('x86_64') || !archs.includes('arm64')) {
        throw new Error(`The universal magequery has ${archs.join(', ') || 'no'} architectures, not both`)
      }
      install(joined, dir, universal)
    }
  } finally {
    rmSync(work, { recursive: true, force: true })
  }
}

async function main(args) {
  const at = args.indexOf('--target')
  const triple = at >= 0 ? args[at + 1] : hostTriple()
  if (!triple) throw new Error('--target needs a target triple')
  await fetchMagequery(triple)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const args = process.argv.slice(2)
  main(args).catch((error) => {
    if (args.includes('--optional') && !(error instanceof Untrusted)) {
      console.warn(`magequery: ${error.message}\nmagequery: going on without it; the Magento wiring view will say so.`)
      return
    }
    console.error(`magequery: ${error.message}`)
    process.exit(1)
  })
}
