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
- [ ] Interactive rebase as a list: reorder, squash, reword, drop
- [ ] "Fix up into this commit" — pick an older commit and autosquash into it
- [ ] Hunk-level and line-level staging, with Stage Hunk / Discard Hunk in the
      diff view
- [ ] Drag a branch onto a remote to push it

### Look and feel
- [x] Lucide icons throughout
- [x] Loading indicators: progress bar, spinners, named in-flight operation
- [x] Slow git work moved off the main thread so the window stops freezing
- [ ] File tree as well as flat path list, with a Path/Tree toggle
- [ ] Side-by-side diff toggle, whitespace-ignore toggle
- [ ] Word-level highlighting inside a changed line
- [ ] Blame and file history views
- [ ] Command palette so every action is reachable by name
- [ ] Keyboard shortcuts for the common actions
- [ ] Conflict panes: synchronised scrolling and syntax highlighting

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
- [ ] Translate common transport failures into something actionable
      ("Permission denied (publickey)" → which key, which agent, what to run)
- [ ] Offer to fix a detached HEAD rather than only reporting it
- [ ] Repository health: stale branches, old stashes, unmerged work
- [ ] Submodules and worktrees
- [ ] Large-repo paging: carry the graph's lane state across pages instead of
      re-walking from the first commit

### Reach
- [ ] A content security policy that works in the bundled app
- [ ] Windows and Linux: the keychain is macOS-only today, and the reveal-in-
      file-manager path needs testing
- [ ] Release build, code signing and notarisation

## Verification

38 tests pass: 7 unit (remote URL parsing, API bases, URL encoding, AI answer
parsing) and 31 integration against real repositories built with the git CLI —
graph lane invariants, divergence reporting, every conflict-resolution
combination, undo and redo, auto-stash, stash operations, empty repository,
detached HEAD, tracking-branch checkout.

The UI is checked by rendering the built bundle in a browser and reading the
console; the Tauri window itself cannot be screenshotted from this shell
(Screen Recording permission), so its layout is reviewed by you.
