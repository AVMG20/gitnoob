# TODO

Every request made in this project, so nothing gets dropped between sessions.
`[x]` done, `[~]` partly done, `[ ]` not started.

## Round 1 — first prototype

- [x] Rust + Tauri desktop app with a Nuxt frontend
- [x] Branch list: local, remote, tags, stashes, with ahead/behind counts
- [x] Commit graph with lane layout
- [x] Merge, push, amend
- [x] Force push with a warning naming the commits that would be dropped
- [x] Merge conflict viewer with checkboxes and three panes
- [x] Fix the blank white window (Nuxt `crossorigin` attributes, and the CSP
      blocking Nuxt's inline import map under `tauri://`)

## Round 2 — layout, integrations, AI

### Layout
- [x] Remove the History / Changes / Conflicts tab row
- [x] Changes always in the right panel
- [x] Top row of the graph is the working tree, selected by default
- [x] Selecting any other commit shows that commit's files
- [x] Conflicts reachable without a tab: from the working-tree row, the toolbar
      banner, or the file list
- [x] Toolbar in three parts: repository left, actions centred, tools right
- [x] Resizable sidebar and right panel, double-click a divider to reset
- [x] Unstaged above staged, sharing the space equally, commit box pinned below
- [x] Clicking a file opens it full width in place of the graph
- [x] Syntax highlighting in Darcula colours, the JetBrains dark scheme

### Projects
- [x] Project tab strip, several repositories open at once, drag to reorder
- [x] Open projects remembered per profile and restored on launch
- [x] Folder button and `+` to open another repository
- [x] Fetch when a project opens, plus an interval, both configurable

### Profiles and forges
- [x] Profile switching, swapping forge, identity and open project tabs
- [x] Global settings versus per-profile settings
- [x] GitHub and GitLab: token check, list pull/merge requests, create one
- [x] Self-hosted hosts, including GitHub Enterprise and nested GitLab groups
- [x] Profile menu in the toolbar showing forge, identity and AI model
- [x] Tokens in the macOS keychain, never in the config file
- [x] Sign-in button opening the forge's token page with scopes pre-selected
- [ ] Real OAuth sign-in (device flow) — needs an application registered with
      GitHub and GitLab, so the client ids have to exist first
- [x] An SSH key per profile: the profile's key becomes a `GIT_SSH_COMMAND`
      with `IdentitiesOnly=yes`, applied at startup and on every profile switch,
      so a work key and a personal key live side by side without editing
      `~/.ssh/config`. Settings lists the pairs found in `~/.ssh` and has a
      "test ssh" button that reads the forge's greeting back, naming the account
- [ ] Match an opened repository's remote host against the profiles and offer
      to switch to the one that owns it
- [ ] Apply the profile identity automatically for repositories under a chosen
      directory, rather than only on request

### AI
- [x] OpenRouter API key in the keychain
- [x] Searchable model picker with prices, context length and sorting
- [x] AI-written commit message from the staged diff
- [x] AI conflict resolution, per region or the whole file
- [ ] AI pull request title and description from the branch's commits
- [ ] Explain this commit / this diff in plain language
- [ ] Suggest how to split an overlarge change into several commits

### Git mechanics
- [x] Stash list with branch, file count and age; apply, pop, drop, branch from
      stash, view a stash's diff
- [x] Stash instead of commit, using the summary as the stash name
- [x] Undo and redo with a history menu, refusing when the branch has moved on
- [x] Auto-stash around branch switches, pulls and rebases, then restore
- [x] Rebase, with abort and continue
- [x] Reset to a commit: soft, mixed or hard, each explained, with a preview of
      what would be lost
- [x] Cherry-pick, revert, tag, delete tag, rename branch, set upstream
- [x] Detect a merge, rebase, cherry-pick or revert in progress

### Direct manipulation
- [x] Drag a branch onto another: fast-forward, merge, or rebase
- [x] Drag a commit onto a branch: cherry-pick onto it
- [x] Drag a stash onto a branch: apply it there
- [x] Drag a branch onto a commit: move the branch there
- [x] Drag files between staged and unstaged
- [x] Right-click menus on commits, branches, remotes, tags, stashes, files and
      pull requests
- [x] Search the graph by message, author or hash, with highlighting and
      next/previous (⌘F, ⌘G)
- [x] Copy hash, short hash, message, patch, path, branch name
- [x] Cherry-pick several commits at once: shift-click and ctrl-click mark rows
      in the graph, and the backend sorts them into history order before handing
      them to git, so picking newest-first still applies oldest-first. Also
      `--no-commit` (stage without committing, to re-split) and `-x` (record
      "cherry picked from")
- [ ] Interactive rebase as a list: reorder, squash, reword, drop
- [ ] "Fix up into this commit" — pick an older commit and autosquash into it
- [x] Hunk-level staging: Stage Hunk, Unstage Hunk and Discard Hunk in the diff
      view, applied by feeding a rebuilt one-hunk patch back to `git apply`
- [ ] Line-level staging (pick individual lines within a hunk)
- [ ] Drag a branch onto a remote to push it

### Round 3 — Windows, keys, and the toolbar as the interactive part
- [x] Amend removed from the toolbar; it lives on the commit box, where the
      message being amended is already in front of you
- [x] A refused push turns the strip under the toolbar into the next step —
      pull with rebase, pull and merge, or force push — rather than a dialog
- [x] Force push always asks a second time, in red, naming the commits that
      stop being reachable. The dead `confirm_force_push` setting is gone: it
      was never read, and it promised a way to skip the question
- [x] Ahead/behind counts use arrow glyphs with a gap, so "↑1" no longer reads
      as "11"
- [x] Context menus put the subject at the bottom, under the actions
- [x] Discard Hunk moved away from Stage Hunk, so the destructive button is not
      where the hand already is
- [x] "Reveal in Finder" says Explorer on Windows
- [x] AI temperature replaced by a thinking level, using OpenRouter's own
      effort scale plus "no thinking", which is the default

### Look and feel
- [x] Lucide icons throughout
- [x] Loading indicators: progress bar, spinners, named in-flight operation
- [x] Slow git work moved off the main thread so the window stops freezing
- [x] File tree as well as flat path list, with a Path/Tree toggle; single-child
      directories are joined into one row so deep trees stay readable
- [ ] Side-by-side diff toggle, whitespace-ignore toggle
- [ ] Word-level highlighting inside a changed line
- [ ] Blame and file history views
- [ ] Command palette so every action is reachable by name
- [ ] Keyboard shortcuts for the common actions
- [ ] Conflict panes: synchronised scrolling and syntax highlighting

## Found by audit, August 2026 — missing outright

These are not refinements of something half-built; the app cannot do them at
all, and none of them were on this list before.

- [ ] Clone a repository. Today the only way in is to open a folder that is
      already a repository, so a machine with no checkout has no route at all.
      This is where the profile work pays off: clone from the work GitLab over
      the work key, from personal GitHub over the personal key, without
      choosing either by hand
- [ ] Create a repository, with a first commit and a `.gitignore`
- [ ] List the repositories a profile's token can see, so cloning is picking
      one from a list rather than pasting a URL. `forge.rs` already
      authenticates and reads pull requests; it has never asked for `/user/repos`
      or `/projects?membership=true`
- [ ] Manage remotes: add one, rename one, change a URL, remove one. `remotes`
      only lists them, and a repository cloned over HTTPS cannot be moved to ssh
      without dropping to the command line
- [ ] Check out a pull request's branch, and read its diff. Reviews can be
      listed and opened in a browser, which is where the app stops
- [ ] Compare two branches directly — what is on one and not the other, as a
      diff rather than as two graph rows
- [ ] Show whether a commit is signed. Signing works today because the git CLI
      does it, but nothing in the window says so, and a profile cannot carry a
      signing key next to its SSH key

## Ideas worth doing, not yet started

### Behind the scenes
- [ ] Warn before committing straight to `main` or `master` — partly done, the
      commit box says so, but it does not yet stop and ask
- [ ] Scan a staged diff for anything that looks like a credential and warn
- [ ] Warn when staging a file large enough to want Git LFS
- [ ] On open, detect an interrupted merge or rebase and offer to finish it
- [ ] Offer to delete local branches whose upstream is gone after a prune
      (the backend already reports them: `stale_branches`)
- [ ] Detect a branch that was squash-merged upstream and offer to clean it up
- [ ] Turn on `rerere` so a conflict resolved once is remembered
- [ ] Auto-set the upstream on a first push rather than failing with a hint
- [ ] Undo beyond this session by reading the reflog
- [ ] Watch the working tree so status updates without a click
- [x] Translate common transport failures into something actionable: `git_cmd`
      appends an explanation to a publickey refusal (naming the pinned key), a
      host-key failure and an HTTPS remote with no credential helper
- [ ] Offer to fix a detached HEAD rather than only reporting it
- [ ] Repository health: stale branches, old stashes, unmerged work
- [ ] Submodules and worktrees
- [ ] Large-repo paging: carry the graph's lane state across pages instead of
      re-walking from the first commit

### Reach
- [ ] A content security policy that works in the bundled app
- [~] Windows and Linux: the keyring feature is now chosen per platform
      (`apple-native`, `windows-native`, `sync-secret-service`), so tokens go to
      the Windows Credential Manager rather than failing to build. The
      reveal-in-file-manager path already branches on Windows but is untested,
      and Linux has never been built.
- [ ] Release build, code signing and notarisation

## How to run it

`npm run app` for development, `npm run app:build` for a real `.app`. Running
`src-tauri/target/debug/gitnoob` by hand gives a blank window: a debug build loads
the `devUrl` from `tauri.conf.json` rather than the bundle compiled into it, so
with no dev server on port 3000 there is nothing to show. Only a release build
serves `frontendDist`. `GITUI_DEVTOOLS=1` opens the inspector, debug builds
only.

`cssCodeSplit` is off so the page links one stylesheet rather than fetching
per-route chunks at runtime.

On Windows the toolchain is rustup with the `x86_64-pc-windows-msvc` host, which
needs the Visual Studio Build Tools (the MSVC x64 compiler and a Windows SDK)
and the WebView2 runtime, which Windows 11 ships already. The test sandbox pins
`core.autocrlf` to false, because Git for Windows turns it on globally and the
tests compare against LF content.

## Verification

59 tests pass on Windows: 22 unit (remote URL parsing, API bases, URL encoding, AI answer
parsing, reasoning levels, one-hunk patch rebuilding, SSH command building, transport-failure
explanations) and 37 integration against real repositories built with the git CLI —
graph lane invariants, divergence reporting, every conflict-resolution
combination, undo and redo, auto-stash, stash operations, cherry-picking several
commits out of order, empty repository, detached HEAD, tracking-branch checkout.

The UI is checked by rendering the built bundle in a browser and reading the
console; the Tauri window itself cannot be screenshotted from this shell
(Screen Recording permission), so its layout is reviewed by you.
