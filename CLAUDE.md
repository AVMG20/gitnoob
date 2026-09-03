# gitnoob

A free, open-source desktop git GUI. Tauri 2 + Rust backend, Nuxt 4 (Vue 3, TypeScript) frontend. GPL-3.0. Ships as `.dmg`, `.exe`/`.msi`, `.AppImage`/`.deb`/`.rpm` from GitHub releases with a self-updater.

What it does: commit graph, line-level staging, drag-and-drop merge/rebase/cherry-pick, interactive rebase and squash, checkbox conflict resolution, GitHub/GitLab PR review inside the app, undo/redo, stash, tags, worktrees, submodules, LFS, profiles (identity + token + SSH key), optional AI commit messages via OpenRouter. Every action logs the git command it ran.

Full details in `docs/development.md`. Read that before touching architecture.

## Commands

```sh
npm install
npm run app                                        # run the app in dev
npm run app:build                                  # release bundle
npm test                                           # vitest (frontend)
npm run typecheck                                  # vue-tsc, must stay clean
cargo test --manifest-path src-tauri/Cargo.toml    # Rust suite
npm run theme                                      # regenerate themes.css from the palette
npm run release <version>                          # cut a release; never bump versions by hand
```

Never launch `src-tauri/target/debug/gitnoob` directly. It shows a blank window.

## Layout

- `src-tauri/src/` Rust backend. `lib.rs` is the Tauri command surface, one file per concern (`work.rs` staging and commits, `remote.rs` fetch/pull/push/merge, `rebase.rs`, `conflict.rs`, `journal.rs` undo, `forge/` GitHub and GitLab, `review.rs` PR roll-up, `config.rs` settings and profiles).
- `src-tauri/tests/repo.rs` integration tests that build real repos with the `git` CLI.
- `app/composables/` frontend state. `useGit.ts` is the single shared store. `useInvoke.ts` wraps every backend call.
- `app/components/` all Vue components. `Dev*Lab.vue` are fixture-backed dev pages, compiled out of release builds.
- `test/` vitest. `*.dom.test.ts` are component tests using happy-dom.
- `scripts/release.mjs` the release script. `scripts/theme/` palette and theme generator.

## Rules

- **Reads use libgit2, writes use the `git` CLI.** Do not add a libgit2 write path or a CLI read path without a reason. The CLI keeps the user's hooks, signing, SSH agent and credential helpers working.
- **Import `invoke` from `~/composables/useInvoke`, never from `@tauri-apps/api/core`.** The wrapper attaches `__repo` so the call targets the right tab's repository.
- **Git arguments are defensive on purpose.** Refs are followed by `--`. Paths are passed as `:(literal)` pathspecs. Pulls always pass `--rebase` or `--no-rebase`. Force push always uses `--force-with-lease`. Keep that pattern for any new command.
- **Do not edit generated files.** `app/assets/css/themes.css` and `app/composables/themeList.ts` come from `scripts/theme/palette.mjs`. Edit the palette and run `npm run theme`. A test fails on drift.
- **Version numbers live in three places** (`tauri.conf.json`, `Cargo.toml`, `Cargo.lock`) and only `npm run release` writes them.
- **CSP changes only apply in release builds.** Verify in a real bundle; a broken policy is a blank window.
- **Windows is a first-class CI target.** Tests must handle CRLF checkouts and cannot use `*` in filenames. Use square brackets to test pathspec escaping.

## Conventions

- Prose in the repo (README, docs, comments) is written plainly and in first person where it fits. No marketing tone.
- Every change that touches git behaviour needs a Rust integration test against a real repo. UI changes get a `*.dom.test.ts`.
- Run `npm run typecheck` before finishing. CI fails on any type error.
- Do not commit or push unless asked.
