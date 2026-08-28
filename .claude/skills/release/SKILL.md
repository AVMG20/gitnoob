---
name: release
description: Cut a release of gitnoob, or bump its version. Use whenever the user asks to release, cut a release, ship a new version, publish an update, or bump the version number — and before running `npm run release` or editing a version by hand.
---

# Releasing gitnoob

One command does the whole thing:

```
npm run release 0.5.3
```

It runs `cargo test` and `npm test`, writes the version into three files,
commits, tags `v0.5.3`, and pushes with `--follow-tags`. Pushing the tag is
what starts `.github/workflows/release.yml`, which drafts a release, builds
macOS, Windows and Linux, and publishes only once all three have uploaded.

Never write a version by hand and never tag by hand. The script is the only
thing that keeps the three files, the commit and the tag agreeing.

## The three files

`src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`.
The tag is what people download, but the number the app reports — and the
number an installed copy compares against when it asks whether an update
exists — comes from `tauri.conf.json`. A tag on a commit that still says the
old number leaves every updated copy being offered the same update forever.

## What it refuses, and why

- **not on main** — pass `--any-branch` if that is really meant
- **the working tree has changes** — commit or stash first; the script commits
  the three version files by name and expects nothing else pending
- **the tag already exists**, locally or on origin — a released version is not
  re-cut, pick the next one
- **origin is ahead** — pull first, because the push would fail after the tag
  was already made
- **the version is not above the current one** — the updater compares these;
  `--force` overrides

That last one has a trap: if the version files were already edited to the new
number, the script reads the new number as the *current* one and refuses. Put
them back to the released number first (`git checkout -- src-tauri/...`) and
let the script do the writing.

## The other flags

- `--dry-run` — every check and both suites, nothing written, committed or
  pushed
- `--skip-tests` — do not run the suites
- `--write-only` — only write the version into the three files, then stop. No
  checks, no commit, no tag, no push. This is what the release workflow uses to
  take the version from the tag it was handed. Running this is **not** a
  release; say so plainly if that is all that was done.

## Choosing the number

Read the current one from `src-tauri/tauri.conf.json` and check what is
actually released with `git ls-remote --tags origin`. The two can disagree: a
number can be bumped in a commit without a tag ever being pushed for it, in
which case that version was never released and its number is still free.

## Before cutting one

The tree must be clean, so any work in progress has to be committed first —
split into topical commits, and ask before committing work that is not yours
to group. Both suites run inside the script, so there is no need to run them
first; a failure there stops the release before anything is committed.
