// One version number, written into the two files that carry it.
//
// The tag is what people download, but the number the app reports — and the
// number the updater compares against — comes from tauri.conf.json and
// Cargo.toml. When those disagree with the tag, an update that has already been
// installed still announces itself as available, forever. So the release
// workflow runs this with the tag it was given before it builds anything.
//
//   node scripts/set-version.mjs 0.2.0
//
// Also useful by hand, to bump both files before tagging.

import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, join } from 'node:path'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')

const version = (process.argv[2] ?? '').replace(/^v/, '')
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`not a version: ${process.argv[2] ?? '(nothing given)'}`)
  console.error('usage: node scripts/set-version.mjs 1.2.3')
  process.exit(1)
}

/** Rewrites one file, saying whether it had to. */
function put(path, find, replace) {
  const full = join(root, path)
  const before = readFileSync(full, 'utf8')
  const was = before.match(find)?.[1]
  if (was === undefined) throw new Error(`no version line found in ${path}`)
  if (was === version) {
    console.log(`${path} already ${version}`)
    return
  }
  writeFileSync(full, before.replace(find, replace))
  console.log(`${path} ${was} → ${version}`)
}

// Both files are edited as text rather than parsed and written back: the
// comments in Cargo.toml and the hand-kept line breaks in the JSON are worth
// more than the tidiness of a round trip. In each, only the first version line
// counts — the ones below it belong to dependencies.
put('src-tauri/tauri.conf.json', /^  "version": "([^"]+)"/m, `  "version": "${version}"`)
put('src-tauri/Cargo.toml', /^version = "([^"]+)"/m, `version = "${version}"`)
