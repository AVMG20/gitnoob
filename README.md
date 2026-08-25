# gitnoob

A desktop Git client for people who would rather not memorise git. Rust and
Tauri underneath, Nuxt (Vue 3) on top.

The idea is that the window teaches as it goes: every git command it runs on
your behalf is printed in the activity log the way you would have typed it, and
anything destructive says what it will cost before it does it.

## What it does

**Getting around.** Open a repository by folder — a subdirectory finds the work
tree root — and keep several open as tabs. Clone one by address, ssh or https,
with the profile's key — or pick it from the list the profile's token can see;
create one with a first commit and a starter `.gitignore` committing as the
profile. A sidebar of local branches with
ahead/behind counts, remote branches, tags and stashes. A commit graph with
topological lanes, ref chips and virtualized rows, so a long history scrolls
without stuttering. Search it by message, author or hash.

**Changing things.** Stage and unstage by file or by hunk, discard, commit,
amend. Fetch, pull, push, merge, rebase, cherry-pick, revert, reset, tag, stash.
Manage the remotes themselves: add one, change its address, rename it, remove it.
Drag a branch onto another to fast-forward, merge or rebase it; drag a commit
onto a branch to cherry-pick; drag a stash onto a branch to apply it there.
Undo and redo, with a history menu that refuses when the branch has moved on.

**Not losing work.** Uncommitted changes are stashed and put back around branch
switches, pulls and rebases — but only when they are actually in the way.
Force-pushing lists the commits that would stop being reachable and asks again,
in red, and always uses `--force-with-lease`. Resetting explains what soft,
mixed and hard each mean and previews what goes. A rejected push turns the strip
under the toolbar into the next step rather than an error.

**Conflicts.** Three panes — ours, theirs, and the result that will be written —
with a fourth for the merge base when `merge.conflictStyle` provides one. A
checkbox per conflict region on each side, a per-region order swap, and
whole-file "take ours/theirs" shortcuts. Marking a file resolved writes it and
stages it.

**Profiles.** A profile carries an identity, a forge, and an SSH key, so a work
account and a personal one live side by side without editing `~/.ssh/config`.
GitHub and GitLab, including Enterprise and self-hosted, can list and open pull
and merge requests. Tokens go to the OS keychain, never the config file.

**AI, optional.** With an OpenRouter key: a commit message from the staged diff,
and conflict resolution per region or per file.

## How it works

Reads go through **libgit2** (the `git2` crate): the revision walk, diffs,
status, ahead/behind counts, ref enumeration. They need to be fast and they need
structured data rather than text to parse.

Writes go through the **`git` CLI**: checkout, add, restore, commit, merge,
fetch, pull, push, stash. That keeps your own environment in force — credential
helpers, SSH agent and `~/.ssh/config`, hooks, commit signing,
`merge.conflictStyle` — none of which libgit2 gets for free. It is also what
makes the activity log honest: it shows the same commands and the same output a
terminal would.

Two consequences worth knowing:

- Where git needs a decision, the app makes it explicitly rather than leaving it
  to config. A pull passes `--rebase` or `--no-rebase`, because a bare `git
  pull` across a divergence stops to ask. Ref arguments are followed by `--`,
  because `git checkout <name>` on a name that is not a ref quietly restores the
  *file* of that name over your edits.
- `git2` is built with `default-features = false`, so there is no `openssl-sys`
  or `libssh2-sys` in the build: the transport those provide is not used.

## Running

Requires Rust and Node.

```sh
npm install
npm run app          # development: starts Nuxt and the window together
npm run app:build    # a real bundle in src-tauri/target/release/bundle
npm run typecheck    # vue-tsc over the frontend
cargo test --manifest-path src-tauri/Cargo.toml
```

**Use one of the first two to run it.** Launching
`src-tauri/target/debug/gitnoob` by hand shows a blank window: a debug build
loads `devUrl` from `tauri.conf.json` — `http://localhost:3000` — rather than
the bundle compiled into it, and with no dev server there is nothing to show.
Only a release build serves `frontendDist`.

`npm run dev` alone serves the frontend in a browser, where every backend call
fails because `invoke` needs the Tauri host. It is still useful for checking
layout. `GITUI_DEVTOOLS=1` opens the web inspector; debug builds only.

Nuxt stamps `crossorigin` on its stylesheet and module script tags, and Tauri
serves the bundle from the `tauri://` protocol where those CORS requests fail —
a blank window with no CSS. A `prerender:generate` hook in `nuxt.config.ts`
strips them. `cssCodeSplit` is off so the page links one stylesheet rather than
fetching per-route chunks at runtime.

On Linux the Tauri build needs the GTK and WebKit development packages
(`libgtk-3-dev`, `libwebkit2gtk-4.1-dev`, `libsoup-3.0-dev`,
`libjavascriptcoregtk-4.1-dev`). On Windows it needs the Visual Studio Build
Tools — the MSVC x64 compiler and a Windows SDK — plus the WebView2 runtime,
which Windows 11 already ships.

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
  conflict.rs   conflict marker parsing and resolution
  work.rs       stage, unstage, discard, commit, amend, stash, hunks
  journal.rs    undo and redo
  config.rs     settings, profiles, projects
  forge.rs      GitHub and GitLab
  ai.rs         OpenRouter
  ssh.rs        per-profile keys
  watch.rs      filesystem watcher
app
  app.vue           shell, tabs, repository picker
  composables/      the single shared store and the invoke wrappers
  components/       sidebar, graph, panels, dialogs, conflict resolver
```

## Testing

`cargo test` runs 147: unit tests over the parts with fiddly rules (remote URL
parsing, one-hunk patch rebuilding, SSH command building, transport-failure
explanations, AI answer parsing, and the config file's round trip, migrations
and corrupt-file path) and integration tests against real repositories built
with the `git` CLI — graph lane invariants, divergence reporting, every
conflict-resolution combination, undo and redo, auto-stash, cherry-picking out
of order, empty repositories, detached HEAD, CRLF files, cloning and creating
repositories, managing remotes against a bare one, and pushing to one, force
push and its lease included.

The frontend has no tests yet, which is the largest gap in the project.
`npm run typecheck` runs, and currently reports 79 errors across seven files —
mostly indexing that Nuxt's strict settings want guarded.

## Known gaps

`TODO.md` is the full list, kept current. The ones worth knowing before you
rely on this:

- **No clone from the forge's own list without a token.** Cloning takes a
    pasted address when there is none; with one, the clone dialog lists what
    the token can see.
- **No content security policy.** `default-src 'self'` blocks Nuxt's inline
  import map, so it is off until a working one is written and checked in the
  bundled app rather than in dev.
- **One repository at a time, underneath.** The window has project tabs, but the
  backend holds a single open path, so a slow operation and a tab switch can
  race.
- No interactive rebase, line-level staging, blame, worktrees, submodules or
  LFS.
- Linux and Windows are built and tested far less than macOS.
