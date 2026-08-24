# gitnoob

A desktop Git client: Rust + Tauri backend, Nuxt (Vue 3) frontend.

## Prototype scope

Working today:

- **Open a repository** by folder, with a recent list. Opening a subdirectory finds the work tree root.
- **Branch sidebar** — local branches with ahead/behind counts, remote branches grouped by remote, tags, stashes. Double-click or the ⤴ button checks out; checking out a remote branch creates a local tracking branch.
- **Commit graph** — topological lane layout with per-lane colours, ref chips, and a virtualized list that only renders the visible rows.
- **Commit detail** — message, author, parents (clickable), changed files with per-file additions/deletions, and an inline diff per file.
- **Changes tab** — staged/unstaged lists, per-file stage/unstage/discard, diff for either side of the index, and a commit box.
- **Fetch / pull / push / merge**, plus stash push and pop.
- **Amend** with a warning when the commit being amended is already on a remote.
- **Force-push guard** — before pushing a diverged branch, the dialog lists the commits that exist only on the remote and would be dropped, and requires an explicit acknowledgement. Force pushes always use `--force-with-lease`.
- **Conflict resolver** — three panes (ours, theirs, and the result that will be written), an optional fourth showing the merge base, a checkbox per conflict region on each side, a per-region order swap, and whole-file "take ours/theirs" shortcuts. Marking a file resolved writes it and stages it.

## Design

Reads go through **libgit2** (the `git2` crate): the revision walk, diffs, status,
ahead/behind counts, and ref enumeration. They need to be fast and they need
structured data rather than text to parse.

Writes go through the **`git` CLI**: checkout, add, restore, commit, merge,
fetch, pull, push, stash. That keeps the user's own environment in force —
credential helpers, SSH agent and `~/.ssh/config`, hooks, commit signing,
`merge.conflictStyle` — none of which libgit2 gets for free. It also means the
activity log at the bottom of the window shows the same output the terminal
would.

Two things about the bundled build are worth knowing. Nuxt stamps `crossorigin`
on its stylesheet and module script tags; Tauri serves the bundle from the
`tauri://` custom protocol, where those CORS requests fail and the window comes
up blank with no CSS and no app. A `prerender:generate` hook in `nuxt.config.ts`
strips them. For the same reason the content security policy is currently off:
`default-src 'self'` blocks Nuxt's inline import map. A working policy needs to
be written and checked in the bundled app, not just in dev.

`git2` is built with `default-features = false`, so there is no `openssl-sys` or
`libssh2-sys` in the build: the transport those provide is not used.

## Running

Requires Rust and Node.

```sh
npm install
npm run app          # development: starts Nuxt and the window together
npm run app:build    # a real .app in src-tauri/target/release/bundle
```

**Use one of those two.** Running `src-tauri/target/debug/gitnoob` by hand shows a
blank window, and the reason is worth knowing: a debug build loads the `devUrl`
from `tauri.conf.json` — `http://localhost:3000` — rather than the bundle
compiled into it. With no dev server listening there is nothing to show. Only a
release build serves the files from `frontendDist`.

If you do want a debug binary that stands on its own, build the frontend first
and drop `devUrl`:

```sh
npm run generate
cargo build --manifest-path src-tauri/Cargo.toml   # with devUrl removed
```

Set `GITUI_DEVTOOLS=1` to open the web inspector on launch; it is only compiled
into debug builds.

`npm run dev` alone serves the frontend in a browser, where every backend call
fails: `invoke` needs the Tauri host. It is still useful for checking layout.

## Layout

```
src-tauri/src
  lib.rs        Tauri command surface
  state.rs      the open repository
  git_cmd.rs    git CLI wrapper
  refs.rs       repo info, branches, tags, stashes, status, checkout
  graph.rs      revision walk and commit-graph lane layout
  diff.rs       commit details, file diffs, working-tree diffs
  remote.rs     fetch, pull, push preview, push, merge
  conflict.rs   conflict marker parsing and resolution
  work.rs       stage, unstage, discard, commit, amend, stash
app
  app.vue           shell, tabs, repository picker
  composables/      the single shared store and the invoke wrappers
  components/       sidebar, graph, panels, dialogs, conflict resolver
```

## Not done yet

- No content security policy — see above.
- Interactive rebase, cherry-pick, revert, reset.
- Hunk- and line-level staging (files stage whole).
- Worktrees, submodules, LFS.
- Pull request / issue integration.
- Conflict panes scroll independently rather than in step.
- The graph reloads from the first commit when you load more history, which gets
  slow past a few tens of thousands of commits; the lane state would need to be
  carried across pages instead.
