# Development

Rust and Tauri underneath, Nuxt (Vue 3) on top. Everything the user-facing
[README](../README.md) leaves out lives here.

## Running

Requires Rust and Node.

```sh
npm install
npm run app          # development: starts Nuxt and the window together
npm run app:build    # a real bundle in src-tauri/target/release/bundle
npm run typecheck    # vue-tsc over the frontend
npm test             # vitest over the frontend
cargo test --manifest-path src-tauri/Cargo.toml
```

**Use one of the first two to run it.** Launching
`src-tauri/target/debug/gitnoob` by hand shows a blank window: a debug build
loads `devUrl` from `tauri.conf.json` rather than the bundle compiled into it,
and with no dev server there is nothing to show. Only a release build serves
`frontendDist`.

`npm run dev` alone serves the frontend in a browser, where every backend call
fails because `invoke` needs the Tauri host. It is still useful for checking
layout, and the labs there draw a page against fixtures rather than against a
repository — `?lab=review` the review page, `?lab=conflict` the resolver, and
`?lab=squash` the commit graph, the working-tree panel and the sidebar together,
which is where squashing, a moved file and the branch menus can be looked at
without arranging a repository into that state first. `&settings=ai` on that one
opens the settings on a section. All three are compiled out of anything built
for release. `GITUI_DEVTOOLS=1` opens the web inspector; debug builds only.

### Platform build dependencies

- **Linux** — `libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`,
  `libjavascriptcoregtk-4.1-dev`.
- **Windows** — the Visual Studio Build Tools (the MSVC x64 compiler and a
  Windows SDK), plus the WebView2 runtime, which Windows 11 already ships.
- **macOS** — Xcode command line tools.

### Two Nuxt quirks worth knowing

Nuxt stamps `crossorigin` on its stylesheet and module script tags, and Tauri
serves the bundle from the `tauri://` protocol where those CORS requests fail —
a blank window with no CSS. A `prerender:generate` hook in `nuxt.config.ts`
strips them. `cssCodeSplit` is off so the page links one stylesheet rather than
fetching per-route chunks at runtime.

## How it works

Reads go through **libgit2** (the `git2` crate): the revision walk, diffs,
status, ahead/behind counts, ref enumeration. They need to be fast and they need
structured data rather than text to parse.

Writes go through the **`git` CLI**: checkout, add, restore, commit, merge,
fetch, pull, push, stash. That keeps the user's own environment in force —
credential helpers, SSH agent and `~/.ssh/config`, hooks, commit signing,
`merge.conflictStyle` — none of which libgit2 gets for free. It is also what
makes the activity log honest: it shows the same commands and the same output a
terminal would.

Two consequences worth knowing:

- Where git needs a decision, the app makes it explicitly rather than leaving it
  to config. A pull passes `--rebase` or `--no-rebase`, because a bare `git
  pull` across a divergence stops to ask. Ref arguments are followed by `--`,
  because `git checkout <name>` on a name that is not a ref quietly restores the
  *file* of that name over your edits. Paths are passed as `:(literal)`
  pathspecs, because a pathspec after `--` still wildmatches, and a file named
  `a[b].txt` would otherwise take `ab.txt` with it.
- `git2` is built with `default-features = false`, so there is no `openssl-sys`
  or `libssh2-sys` in the build: the transport those provide is not used.

## Which repository a call is about

The window has project tabs; the backend holds one open path. Every call from
the window carries `__repo` — the repository it believes it is asking about —
and one wrapper around the invoke handler (`aimed`, at the foot of `lib.rs`)
applies it before the command runs. The frontend side is
`app/composables/useInvoke.ts`, which every `invoke` in the app goes through.

**Import `invoke` from `useInvoke`, never from `@tauri-apps/api/core`.** A
call that skips the wrapper is not addressed to anything, and will act on
whichever repository happens to be open when it lands.

## Layout

```
src-tauri/src
  lib.rs        Tauri command surface
  state.rs      the open repository
  git_cmd.rs    git CLI wrapper, and the command log the window shows
  refs.rs       repo info, branches, tags, stashes, status, checkout
  create.rs     clone and create: bringing a repository into existence
  graph.rs      revision walk and commit-graph lane layout
  diff.rs       commit details, file diffs, working-tree diffs
  remote.rs     fetch, pull, push preview, push, merge, rebase
  rebase.rs     the interactive rebase, and squashing a run of commits
  conflict.rs   conflict marker parsing and resolution
  work.rs       stage, unstage, discard, commit, amend, stash, hunks
  worktree.rs   listing, adding and removing worktrees
  journal.rs    undo and redo
  config.rs     settings, profiles, projects
  forge/        GitHub and GitLab
    mod.rs        the calls a command makes
    types.rs      what a forge hands back
    http.rs       account, host, request, pagination
    people.rs     faces, fetched once and cached
    github.rs     what is GitHub's alone
    gitlab.rs     what is GitLab's alone
  review.rs     one pull or merge request, rolled up
  ai.rs         OpenRouter
  ssh.rs        per-profile keys
  avatar.rs     author pictures, fetched once and cached
  watch.rs      filesystem watcher
app
  app.vue           shell, tabs, repository picker
  composables/      the single shared store and the invoke wrappers
  components/       sidebar, graph, panels, dialogs, conflict resolver, review
  assets/css/       main.css and the generated themes.css
scripts
  release.mjs       the one command that cuts a release
  theme/            the palette, and the generator that writes themes.css
```

## Themes

`app/assets/css/themes.css` and `app/composables/themeList.ts` are **generated**
from `scripts/theme/palette.mjs` by `npm run theme`. Edit the palette, not the
output — a test regenerates both and fails if what is checked in has drifted,
and the same test checks every theme at every contrast level against the
contrast ratios each piece of text has to meet.

## The content security policy

`app.security.csp` in `tauri.conf.json` is `default-src 'self'`, with
`img-src` opened to `data:` and `https:` (author pictures arrive as data
URIs; a request's description can hold a screenshot) and `connect-src` opened
to `ipc: http://ipc.localhost`, which is how a call reaches the backend at
all.

Nuxt's inline import map is what kept this off for a long time. It still cannot
be an external file — an import map has to be inline — but `tauri-build`
hashes the inline scripts of the bundled page at build time and adds those
hashes to the policy, so they are allowed by hash rather than by putting
`'unsafe-inline'` back.

The policy only applies to a **release** build, since a debug build loads the
page from the dev server. Changing it means checking it in a real bundle: a
policy that blocks the app's own scripts shows as a blank window.

## Testing

```sh
cargo test --manifest-path src-tauri/Cargo.toml   # 373
npm test                                          # 398 across 47 files
npm run typecheck
```

The Rust suite is unit tests over the parts with fiddly rules (remote URL
parsing, API bases, check and verdict states, the merge-readiness roll-up,
one-hunk patch rebuilding, SSH command building, transport-failure explanations,
AI answer parsing, and the config file's round trip, migrations and corrupt-file
path) and integration tests against real repositories built with the `git` CLI —
graph lane invariants, divergence reporting, every conflict-resolution
combination, undo and redo, auto-stash, cherry-picking out of order, squashing a
run of commits and undoing it, files moved with `git mv` and files moved with
the shell, staged, unstaged, discarded and read as a diff,
empty repositories, detached HEAD, CRLF files, cloning and creating
repositories, managing remotes against a bare one, and pushing to one, force
push and its lease included.

The frontend suite covers the review page end to end against a fixture forge,
the markdown renderer, the patch parser, the conflict grid, the diff and graph
views, the branch-deletion verdicts, the ref chips, the squash dialog and the
menus that open it, the highlighter and the themes.

Two notes for anyone running the suites on Windows:

- A checkout writes CRLF, so a test comparing a file against a fixture written
  with bare newlines has to normalise first. The theme test does.
- `*` cannot be part of a filename, so the pathspec test that needs one is
  `#[cfg(not(windows))]`; the same hazard is covered everywhere by a name with
  square brackets in it.

`npm run typecheck` reports nothing, and the checks workflow fails if that
changes.

## Releasing

One command:

```sh
npm run release 0.5.0
```

It runs both suites and stops if either fails, writes the version into
`tauri.conf.json`, `Cargo.toml` and `Cargo.lock`, commits that, tags it and
pushes — and refuses before any of it on a dirty tree, a branch that is not
main, a tag that already exists here or on the remote, or a version that is not
above the current one. `--dry-run` runs every check and both suites and changes
nothing; `--skip-tests`, `--any-branch` and `--force` lift one check each.

The push of the `v*` tag is what starts `.github/workflows/release.yml`, which
builds the app for all three platforms and publishes a GitHub release with the
installers attached, so nobody has to install Rust to run it.

What comes out: a universal `.dmg` for macOS covering Intel and Apple Silicon,
an NSIS `.exe` and an `.msi` for Windows, and an `.AppImage`, a `.deb` and an
`.rpm` for Linux — the Linux build on Ubuntu 22.04 rather than the newest
release, because the AppImage carries the glibc it was built against as a floor.

A draft release is created before the three build jobs start, so they have an
agreed place to upload to rather than three of them each creating "the"
release, and it is published only once all three have finished. A release is
never half a release.

The version compiled into the app is taken from the tag before anything is
built — the workflow runs `scripts/release.mjs --write-only`, the same writing
the release command does by hand, so the two cannot drift. It matters more than
it looks: the updater compares that number against the newest release, and an
app that reports a version older than the one it is would offer to install
itself, for ever.

### Signing

Bundles are signed with the project's updater key, and both halves of it must be
dealt with once:

```sh
npx tauri signer generate -w ~/.tauri/gitnoob.key
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/gitnoob.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body ""
```

The public half lives in `tauri.conf.json` under `plugins.updater.pubkey` and is
already there. **Keep the private key.** It is not in the repository and cannot
be recovered from anything that is; lose it and every installed copy refuses
every future update, and everyone reinstalls by hand.

With the secrets missing the build fails rather than publishing unsigned
bundles, which is the intended failure: an unsigned bundle is one every
installed copy would refuse anyway.

Nothing is code-signed with an Apple or Windows certificate yet, so a first
launch needs a nudge. Each release says how to get past it.

### Updating in place

Settings → Updates: the version installed, a button to check, and the release
notes of whatever is on offer before the user agrees to it. Installing downloads
the bundle, verifies its signature against the public key compiled into the
running app, writes it and restarts. On Linux this works from the AppImage;
installed from the `.deb` or `.rpm`, updating belongs to the package manager.

The app also asks once at launch, quietly — a machine that is offline should not
be told so every time the window opens — and what it finds turns up as a line in
the profile menu and a dot beside Updates in settings. "Look for a new version at
launch" in that same page turns it off.

## Screenshots

The README's images live in [`screenshots/`](screenshots/), which has its own
note on what each one is meant to show and how to take a replacement.
