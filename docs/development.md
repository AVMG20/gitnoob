# Development

Tauri 2 and Rust underneath, Nuxt 4 (Vue 3) on top. This is the doc for people hacking on gitnoob. The [README](../README.md) covers install and the feature list.

## Running

You need Node and Rust.

```sh
npm install
npm run app          # dev: Nuxt dev server plus the Tauri window
npm run app:build    # release bundle in src-tauri/target/release/bundle
npm run typecheck    # vue-tsc
npm test             # vitest
cargo test --manifest-path src-tauri/Cargo.toml
```

Always run the app through `npm run app` or a release build. Launching `src-tauri/target/debug/gitnoob` by hand gives a blank window, because a debug build loads the dev server URL from `tauri.conf.json` and there is none running.

`npm run dev` on its own serves the frontend in a browser. Every backend call fails there because `invoke` needs Tauri, but it is handy for layout work. There are also a few fixture-backed lab pages that let you look at a screen without setting up a repo in that state:

- `?lab=review` the pull request page
- `?lab=conflict` the conflict resolver
- `?lab=home` the home tab (`&slow=1200` delays the load so you can see the skeleton)
- `?lab=squash` the graph, working tree panel and sidebar together (`&settings=ai` opens settings on a section)

The labs are compiled out of release builds. `GITUI_DEVTOOLS=1` opens the web inspector in debug builds.

### Platform build dependencies

- **Linux**: `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`, `libjavascriptcoregtk-4.1-dev`
- **Windows**: Visual Studio Build Tools (MSVC x64 and a Windows SDK) plus the WebView2 runtime
- **macOS**: Xcode command line tools

### Two Nuxt quirks

Nuxt puts `crossorigin` on its stylesheet and script tags. Tauri serves the bundle from `tauri://`, where those CORS requests fail and you get a blank window with no CSS. A `prerender:generate` hook in `nuxt.config.ts` strips the attribute. `cssCodeSplit` is off so the page links one stylesheet instead of fetching per-route chunks at runtime.

## How it works

**Reads go through libgit2** (the `git2` crate): revision walk, diffs, status, ahead/behind, refs. Fast, and structured data instead of text to parse.

**Writes go through the `git` CLI**: checkout, add, restore, commit, merge, fetch, pull, push, stash. This keeps the user's environment in force: credential helpers, SSH agent, `~/.ssh/config`, hooks, commit signing, `merge.conflictStyle`. It also makes the activity log honest, since it shows the exact commands a terminal would run.

Two things that follow from this:

- Where git would stop and ask, the app decides explicitly. A pull passes `--rebase` or `--no-rebase`. Ref arguments are followed by `--`, because `git checkout <name>` on a non-ref restores the file of that name over your edits. Paths are passed as `:(literal)` pathspecs, because a pathspec after `--` still wildmatches and `a[b].txt` would otherwise match `ab.txt`.
- `git2` is built with `default-features = false`. No `openssl-sys`, no `libssh2-sys`, because that transport is never used.

## Which repo a call is about

The window has project tabs. The backend holds one open path. Every call from the frontend carries `__repo`, the repository it thinks it is talking to, and the `aimed` wrapper at the bottom of `lib.rs` applies it before the command runs. On the frontend that is `app/composables/useInvoke.ts`.

**Import `invoke` from `useInvoke`, never from `@tauri-apps/api/core`.** A call that skips the wrapper acts on whichever repo happens to be open when it lands.

## Layout

```
src-tauri/src
  lib.rs        Tauri command surface
  state.rs      the open repository
  git_cmd.rs    git CLI wrapper and the command log
  refs.rs       repo info, branches, tags, stashes, status, checkout
  create.rs     clone and init
  graph.rs      revision walk and lane layout
  diff.rs       commit details, file diffs, working tree diffs
  blame.rs      blame
  remote.rs     fetch, pull, push, merge, rebase
  rebase.rs     interactive rebase and squash
  conflict.rs   conflict marker parsing and resolution
  work.rs       stage, unstage, discard, commit, amend, stash, hunks
  worktree.rs   worktrees
  submodule.rs  submodules
  lfs.rs        git-lfs
  journal.rs    undo and redo
  home.rs       the home tab
  config.rs     settings, profiles, projects
  forge/        GitHub and GitLab
  review.rs     one pull or merge request, rolled up
  ai.rs         OpenRouter
  ssh.rs        per-profile keys
  sign.rs       commit signing
  avatar.rs     author pictures, cached
  watch.rs      filesystem watcher
src-tauri/tests
  repo.rs       integration tests against real repos built with git
app
  app.vue           shell, tabs, repo picker
  composables/      useGit.ts is the shared store; useInvoke.ts wraps every call
  components/       everything visual
  assets/css/       main.css and the generated themes.css
test                vitest, unit and happy-dom component tests
scripts
  release.mjs       cuts a release
  theme/            palette and the generator for themes.css
```

## Themes

`app/assets/css/themes.css` and `app/composables/themeList.ts` are generated from `scripts/theme/palette.mjs` by `npm run theme`. Edit the palette, not the output. A test regenerates both and fails if the checked-in files drifted, and checks every theme against WCAG contrast ratios.

## Content security policy

`app.security.csp` in `tauri.conf.json` is `default-src 'self'`, with `img-src` opened to `data:` and `https:` for avatars and screenshots in PR descriptions, and `connect-src` opened to `ipc: http://ipc.localhost` so calls can reach the backend at all.

Nuxt's inline import map has to stay inline. `tauri-build` hashes the inline scripts at build time and adds the hashes to the policy, which is how they are allowed without `'unsafe-inline'`.

The policy only applies to release builds. If you change it, check it in a real bundle: a policy that blocks the app's own scripts shows as a blank window.

## Testing

```sh
cargo test --manifest-path src-tauri/Cargo.toml
npm test
npm run typecheck
```

The Rust suite is unit tests for the fiddly parsers and roll-ups, plus integration tests that build real repositories with the `git` CLI: graph lanes, divergence, every conflict combination, undo and redo, auto-stash, cherry-pick, squash, moved files, empty repos, detached HEAD, CRLF, clone and init, remotes against a bare repo, push and force push.

The frontend suite covers the review page against a fixture forge, the markdown renderer, patch parser, conflict grid, diff and graph views, branch deletion, ref chips, squash dialog, highlighter and themes. Component tests declare `@vitest-environment happy-dom` at the top of the file.

CI runs both suites on all three platforms on every push and PR (`.github/workflows/check.yml`). Two Windows notes:

- A checkout writes CRLF, so tests comparing against fixtures with bare newlines normalise first.
- `*` is not a valid filename on Windows, so the pathspec test that needs one is `#[cfg(not(windows))]`. The square-bracket case covers the same hazard everywhere.

`npm run typecheck` reports nothing today and CI fails if that changes.

## Releasing

```sh
npm run release 0.5.0
```

It runs both suites, writes the version into `tauri.conf.json`, `Cargo.toml` and `Cargo.lock`, commits, tags and pushes. It refuses on a dirty tree, a branch that is not main, an existing tag, or a version that is not above the current one. `--dry-run` runs every check and changes nothing. `--skip-tests`, `--any-branch` and `--force` each lift one check.

Pushing the `v*` tag starts `.github/workflows/release.yml`, which builds all three platforms and publishes a GitHub release with the installers attached. A draft release is created first so the three jobs have one place to upload to, and it is published only when all three finish.

The workflow runs `scripts/release.mjs --write-only` before building so the version compiled into the app matches the tag. This matters: the updater compares that number against the newest release, and an app that reports an older version than it is would offer to install itself forever.

What comes out: a universal `.dmg`, an NSIS `.exe` and an `.msi`, and an `.AppImage`, `.deb` and `.rpm`. Linux builds on Ubuntu 22.04 on purpose, because the AppImage carries the glibc it was built against as a floor.

### Signing

Bundles are signed with the project's updater key:

```sh
npx tauri signer generate -w ~/.tauri/gitnoob.key
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/gitnoob.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body ""
```

The public half is in `tauri.conf.json` under `plugins.updater.pubkey`. **Do not lose the private key.** It is not in the repo and cannot be recovered. Without it every installed copy refuses every future update.

With the secrets missing, the build fails instead of publishing unsigned bundles. That is intended.

Nothing is code-signed with an Apple or Windows certificate yet, so first launch needs a click past the warning.

### Updating in place

Settings → Updates shows the installed version, a check button, and the release notes of whatever is on offer. Installing downloads the bundle, verifies the signature against the compiled-in public key, writes it and restarts. On Linux this works from the AppImage; `.deb` and `.rpm` installs update through the package manager.

The app also checks once at launch, quietly. A new version shows as a line in the profile menu and a dot next to Updates in settings. "Look for a new version at launch" in that page turns it off.

## Screenshots

The README's images live in [`screenshots/`](screenshots/).
