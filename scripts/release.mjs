// Cut a release: set the version, commit it, tag it, push.
//
//   npm run release 0.3.0
//
// Pushing a `v*` tag is what starts the workflow in .github/workflows — this
// script exists because the steps around that tag have to happen in one order
// and none of them can be left out. The tag is what people download, but the
// number the app reports, and the number an installed copy compares against
// when it asks whether an update exists, comes from tauri.conf.json and
// Cargo.toml. Tag a commit that still says the old number and every copy that
// has already taken the update keeps being offered it, forever.
//
// The suites run first and a failure ends it: a release is the one build
// nobody gets to try before they download it.
//
// It refuses rather than repairs. A dirty tree, a branch that is not main, a
// tag that already exists, a version that goes backwards: each of those is a
// person having meant something else, and none is worth guessing at halfway
// through a push.
//
//   --dry-run       run every check and both suites, change nothing
//   --skip-tests    do not run the suites
//   --any-branch    release from somewhere other than main
//   --force         allow a version that is not above the current one
//   --write-only    only write the version into the files, and stop
//
// The release workflow uses `--write-only` to take the version from the tag it
// was handed before it builds anything, which is the same writing this script
// does by hand — one place, so the two cannot drift.

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

const args = process.argv.slice(2)
const flags = new Set(args.filter((arg) => arg.startsWith('--')))
const version = (args.find((arg) => !arg.startsWith('--')) ?? '').replace(/^v/, '')
const tag = `v${version}`
const dry = flags.has('--dry-run')

/** Says why it will not go on, and goes no further. */
function stop(what, hint) {
  console.error(`\n  ${what}`)
  if (hint) console.error(`  ${hint}`)
  console.error('')
  process.exit(1)
}

const known = ['--dry-run', '--skip-tests', '--any-branch', '--force', '--write-only']
for (const flag of flags) {
  if (!known.includes(flag)) stop(`unknown option ${flag}`, `known: ${known.join(' ')}`)
}

if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  stop(`not a version: ${args[0] ?? '(nothing given)'}`, 'usage: npm run release 1.2.3')
}

// --- the version, in the three files that carry it

/** Rewrites one file, saying whether it had to. A rehearsal only says. */
function put(path, find, replace) {
  const full = join(root, path)
  const before = readFileSync(full, 'utf8')
  const was = before.match(find)?.[1]
  if (was === undefined) stop(`no version line in ${path}`, 'the file is not the shape this expects')
  if (was === version) {
    console.log(`  ${path} already ${version}`)
    return
  }
  if (!dry) writeFileSync(full, before.replace(find, replace))
  console.log(`  ${path} ${was} → ${version}${dry ? ' (not written)' : ''}`)
}

/**
 * Writes the version everywhere it is kept.
 *
 * The files are edited as text rather than parsed and written back: the
 * comments in Cargo.toml and the hand-kept line breaks in the JSON are worth
 * more than the tidiness of a round trip. In each, only this package's own
 * version line counts — every other one belongs to a dependency.
 *
 * The lock file is here because cargo rewrites it on the next build whether or
 * not anybody asked. Left out, the release commit is followed by a lock file
 * that no longer matches it, and in CI by a dirty tree.
 */
const VERSIONED = ['src-tauri/tauri.conf.json', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock']

function writeVersion() {
  put('src-tauri/tauri.conf.json', /^ {2}"version": "([^"]+)"/m, `  "version": "${version}"`)
  put('src-tauri/Cargo.toml', /^version = "([^"]+)"/m, `version = "${version}"`)
  put('src-tauri/Cargo.lock', /(?<=name = "gitnoob"\n)version = "([^"]+)"/, `version = "${version}"`)
}

/** The number the app currently reports, which the new one has to beat. */
function currentVersion() {
  return readFileSync(join(root, 'src-tauri/tauri.conf.json'), 'utf8').match(
    /^ {2}"version": "([^"]+)"/m
  )?.[1]
}

// What the workflow asks for: the files, and nothing else. None of the checks
// below apply to it — it is on a detached tag, by definition not on main.
if (flags.has('--write-only')) {
  writeVersion()
  process.exit(0)
}

// --- running things

/**
 * Runs a command with its output on show, and stops here if it fails.
 *
 * Without the catch, a failing suite ends in a stack trace out of node's own
 * internals, which says nothing about what went wrong and reads like a bug in
 * this script rather than a test that did not pass.
 */
function run(command, ...rest) {
  try {
    execFileSync(command, rest, { cwd: root, stdio: 'inherit' })
  } catch {
    stop(`${command} ${rest.join(' ')} failed`, 'nothing has been committed, tagged or pushed')
  }
}

/** The same, for the steps that change something: a dry run only names them. */
function mutate(command, ...rest) {
  if (dry) {
    console.log(`  would run: ${command} ${rest.join(' ')}`)
    return
  }
  run(command, ...rest)
}

/** Runs a command for what it says rather than for what it does. */
function read(command, ...rest) {
  return execFileSync(command, rest, { cwd: root, encoding: 'utf8' }).trim()
}

const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'

// --- the checks

const branch = read('git', 'rev-parse', '--abbrev-ref', 'HEAD')
if (branch !== 'main' && !flags.has('--any-branch')) {
  stop(`on ${branch}, not main`, 'release from main, or pass --any-branch')
}

if (read('git', 'status', '--porcelain')) {
  stop('the working tree has changes', 'commit or stash them first — this script commits')
}

if (read('git', 'tag', '--list', tag)) {
  stop(`${tag} already exists here`, 'a released version is not re-cut; pick the next one')
}

// The remote is the one that matters: a tag deleted locally is still a release.
if (read('git', 'ls-remote', '--tags', 'origin', tag)) {
  stop(`${tag} already exists on origin`, 'that release has been made')
}

// A push that is behind fails anyway, and it fails after the tag has been made.
run('git', 'fetch', '--quiet', 'origin', branch)
const behind = read('git', 'rev-list', '--count', `HEAD..origin/${branch}`)
if (behind !== '0') {
  stop(`${behind} commit(s) on origin/${branch} are not here`, 'pull first')
}

/** Compares two versions as numbers, so 0.10.0 sits above 0.9.0. */
function ordered(from, to) {
  const parts = (one) => one.split('-')[0].split('.').map(Number)
  const [a, b] = [parts(from), parts(to)]
  for (let at = 0; at < 3; at++) {
    if ((b[at] ?? 0) !== (a[at] ?? 0)) return (b[at] ?? 0) > (a[at] ?? 0)
  }
  // Same numbers: a prerelease going to the release proper is still forwards.
  return from.includes('-') && !to.includes('-')
}

const current = currentVersion()
if (current && !ordered(current, version) && !flags.has('--force')) {
  stop(
    `${version} is not above ${current}`,
    'the updater compares these; pass --force if you mean it'
  )
}

// --- the release

console.log(`\n  ${current} → ${version}${dry ? '  (dry run)' : ''}\n`)

if (!flags.has('--skip-tests')) {
  // Run even under --dry-run: they change nothing, and a rehearsal that skipped
  // the one step that can fail would not be worth running.
  console.log('  Tests\n')
  run('cargo', 'test', '--quiet', '--manifest-path', 'src-tauri/Cargo.toml')
  run(npm, 'test')
}

console.log('\n  Version\n')
writeVersion()

console.log('')
// The paths are named rather than swept up with -a: the tree was clean when
// this started, and naming them keeps that true if it stops being.
mutate('git', 'commit', '--quiet', '-m', `Version ${version}`, ...VERSIONED)
mutate('git', 'tag', '-a', tag, '-m', `gitnoob ${version}`)

// --follow-tags carries the annotated tag along with the commit, so the two
// arrive together: a tag that lands on a commit the remote has not seen starts
// a build of something nobody can look at.
console.log('\n  Push\n')
mutate('git', 'push', '--follow-tags', 'origin', branch)

if (dry) {
  console.log('\n  Nothing was changed. Drop --dry-run to do it.\n')
  process.exit(0)
}

// A remote that is not GitHub has no pages to point at.
const repo = read('git', 'remote', 'get-url', 'origin')
  .replace(/^git@github\.com:/, 'https://github.com/')
  .replace(/\.git$/, '')
const pages = repo.startsWith('https://github.com/')
  ? `\n  ${repo}/actions\n  ${repo}/releases/tag/${tag}\n`
  : ''

console.log(`
  ${tag} is on its way.

  The workflow drafts the release, builds macOS, Windows and Linux, and
  publishes only once all three have uploaded.
${pages}`)
