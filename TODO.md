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
- [x] The gutter marks in the file view answer for themselves: clicking the
      bar beside a changed line shows what that line said before, and clicking
      the wedge at a seam shows the lines that were deleted there. The old text
      is coloured with the same grammar as the file, sits under the line that
      replaced it (or above the seam the deletions were at), and scrolls inside
      itself past a third of the window rather than pushing the file around.
      Escape or a click anywhere else closes it. This meant keeping the deleted
      text rather than only counting it — and while doing so, a run where more
      lines went than came back now accounts for the surplus instead of dropping
      it
- [ ] Side-by-side diff toggle, whitespace-ignore toggle
- [ ] Word-level highlighting inside a changed line
- [ ] Blame and file history views
- [ ] Command palette so every action is reachable by name
- [x] Keyboard shortcuts for the common actions: fetch, pull, push, refresh,
      new branch, stash, undo, redo, settings, open a repository, close a
      project tab, and the nine project tabs by position. `useShortcuts.ts`
      holds the bindings and their descriptions in one list, and the Shortcuts
      page in settings renders that same list — so a key that changes changes in
      one place, and the page cannot drift from what the window listens for
- [x] Resizable columns in the commit list: drag the line between two headings,
      double-click it to put it back. Which columns are drawn is a choice too —
      right-click the headings, or use Appearance in settings. Widths and
      visibility live in the browser's own storage next to the theme, because
      they are the machine's preference rather than the profile's
- [x] Long branch names are cut in the middle rather than at the end. Four chips
      reading `origin/ASANA-1216293…` are the same chip as far as the eye is
      concerned; the digits that tell them apart are at the end. Two spans do it
      — the head shrinks and takes the ellipsis, the tail never shrinks — so the
      cut lands where the real width puts it and nothing is measured
- [ ] Conflict panes: synchronised scrolling and syntax highlighting

## Round 4 — the code audit, August 2026

### Done in this pass

- [x] **Resolving a conflict no longer rewrites a CRLF file to LF.** The parser
      splits on `str::lines`, which eats the carriage returns, and the result
      was rejoined with `\n` — so resolving one conflict in a Windows-line-ended
      file showed every line in it as changed. The file's own ending is detected
      and put back. Never seen because the test sandbox pins `core.autocrlf` to
      false, which is exactly the config that hides it
- [x] **`git checkout <name>` is followed by `--`.** Without it a name that is
      not a ref is read as a path, and `git checkout notes.txt` silently
      restores that file over the uncommitted work in it. Now it is an error,
      which is the truth. Applies to checkout, tracking checkout, branch
      creation, and the checkouts inside pull and undo
- [x] **A pull says which reconciliation it means.** A bare `git pull` across a
      divergence dies with "Need to specify how to reconcile divergent
      branches" unless `pull.rebase` is configured, which nobody opening this
      app has done. `--rebase` or `--no-rebase` is always passed. The UI had
      always asked the question; it just was not passing the answer on
- [x] **The activity log shows the git command that ran**, written the way it
      would be typed. `CmdOutput` had carried the argv all along and the window
      threw it away. Pure queries are left out, so the log is a record of what
      changed the repository. This is the cheapest teaching the app can do
- [x] **A failed read is reported.** The six calls behind a refresh each
      swallowed their errors, so a status that had stopped updating looked
      exactly like a repository that had stopped changing
- [x] **Watcher events that arrive mid-operation are kept, not dropped.** They
      were discarded whenever the app was busy, and the next event might be
      minutes away — long enough to sit looking at a stale window. They are now
      applied when the work finishes
- [x] **Diffs are capped at 10,000 lines**, with the remainder counted and
      named in the view. A regenerated lockfile was collected in full, sent as
      JSON, and given a DOM node per line
- [x] **Syntax highlighting is cached per line.** It ran from the template, so
      highlight.js re-ran for every line of a diff on every re-render
- [x] `npm run typecheck`, and `store.busy` typed rather than invisible to
      TypeScript — it is attached with `defineProperty`, so all 39 uses of it
      were errors nobody was running the checker to see

### Next, in the order I would take them

- [ ] **One repository at a time, underneath.** `AppState` holds a single path
      and not one of the 89 commands takes a repository argument, but the window
      has project tabs, a watcher and an interval fetch. A tab switch during an
      operation can land a read — or a write — on the wrong repository. Either
      thread a repository handle through the command surface, or carry an epoch
      that stale replies are dropped against
- [ ] **A content security policy.** `csp` is `null` and the diff view uses
      `v-html`. highlight.js escapes what it emits, so there is no way in today,
      but in a window with 89 commands one injection is arbitrary git
      execution with the user's key. The policy has to be written against the
      bundled app, not dev
- [ ] **Frontend tests.** 9,400 lines and no runner. Vitest with a mocked
      `invoke` would cover the store, and `GraphList`'s virtualization and the
      drag-drop target rules are the parts most likely to break silently
- [x] **Tests for `config.rs`.** 289 lines carrying profiles, projects, the
      rename-on-corrupt path and a migration, with no test over any of it — and
      losing it loses the user's whole setup
      — done: ten tests over a temporary config directory. A first run, a save
      and its round trip, the temp file not being left behind, a corrupt file
      moved aside byte for byte rather than overwritten, a config written before
      ssh keys and avatars existed still yielding its profiles, the copy from
      the old bundle name happening once and never over an existing file, and
      the active profile resolving — including to nothing, when the id names a
      profile that was deleted
- [ ] **Tests for the forge and AI HTTP paths.** Only response parsing is
      covered. Nothing exercises a 401, a rate limit, an Enterprise base URL, a
      nested GitLab group, or pagination. A mock server would fix that
- [x] **A test for `push` itself.** `push_preview` is well covered; the push it
      previews, including `--force-with-lease`, is never run against a bare
      remote, though the test harness already builds one
      — done: four tests against a bare origin, reading the remote's own refs
      rather than the push's exit code. A first push sets the upstream and
      lands; a non-fast-forward push is refused and leaves the remote where it
      was; a force push carries `--force-with-lease` and never a bare `--force`;
      and the lease refuses a force push over a remote that moved since the last
      fetch, leaving the other person's commit as the tip
- [ ] **Clear the 59 remaining typecheck errors** (`SideBar`, `GraphList`,
      `WorkingChanges`, `ConflictView`, `CommitDetails`, `DeleteBranchDialog`),
      then make the checker a gate
- [ ] **Undo that outlives the session.** The journal is in memory, so quitting
      loses it. The reflog is already on disk
- [ ] **A safety net for discard.** It is the most destructive button a
      beginner presses and the one operation the journal does not record.
      Discarded changes could go to a hidden ref, recoverable for a few days
- [ ] **Accessibility.** Not one `aria-` attribute in the app, and no arrow-key
      navigation of the commit list or the file list
- [~] **Continuous integration.** `.github/workflows/release.yml` builds all
      three platforms on a tag, so a break that only shows up on Linux or
      Windows is caught by the next release rather than by a user. Nothing yet
      runs on a push: `cargo test`, `cargo clippy` and `npm run typecheck`
      still guard nothing
- [ ] Conflict resolution reads the file with `read_to_string`, so a file that
      is not valid UTF-8 fails rather than saying why

## Round 5 — shipping it

Building it yourself was the only way to run it. Now a tag is.

- [x] **A pipeline that builds the app on a tag.** `.github/workflows/release.yml`
      drafts a release, builds macOS, Windows and Linux in parallel, uploads to
      that draft, and publishes it only when all three have finished — so a
      release is never half a release and never three of them. macOS is one
      universal bundle rather than two downloads and a question about which
      Mac you have; Linux builds on 22.04, because the AppImage carries the
      glibc it was built against as a floor and building on the newest Ubuntu
      quietly excludes everyone on an older one
- [x] **The version comes from the tag.** `scripts/release.mjs --write-only`
      writes it into `tauri.conf.json`, `Cargo.toml` and `Cargo.lock` before the
      build. The updater compares that number against the newest release, so an
      app reporting a version older than the one it is would offer to install
      itself, for ever
- [x] **One command cuts a release.** `npm run release 0.3.0` runs both suites,
      writes the version, commits, tags and pushes, and refuses before any of it
      on a dirty tree, a branch that is not main, a tag that already exists here
      or on the remote, or a version that does not go forwards. `--dry-run` is
      the whole thing including the suites, changing nothing. The steps used to
      be four lines in a comment, which is three too many to get right at the
      moment you want them
- [x] **Update from inside the window.** Settings → Updates: the installed
      version, a check button, the release notes of what is on offer, and one
      button that downloads it, verifies the signature against the public key
      compiled into the running copy, writes it and restarts. A quiet check at
      launch — off in one click — surfaces as a line in the profile menu and a
      dot beside Updates, rather than as a dialog over the repository you came
      to look at
- [x] **Tags, which the release flow runs on.** Three things were wrong at
      once, and all three were the same thing: `git tag -a` writes an object of
      its own, and its id is not the commit's. The tag list handed that id
      straight out, so clicking a tag asked the graph for a commit that does
      not exist; the graph decoration hung the chip on it, so an annotated tag
      appeared nowhere in the chart. Both peel now. On top of that: clicking a
      tag reveals its commit, the way clicking a branch always has; an empty
      Tags list says how to make one rather than only that there are none — a
      + on the header would have had to guess which commit you meant, and
      right-clicking the commit never does; the new-tag dialog pushes to origin
      unless you say otherwise, because a tag only origin has not seen starts
      no release and tells nobody anything; and a tag row shows its date, with
      its message and kind on hover. The list is newest first
      rather than alphabetical, because sorting release tags by name puts
      v0.10.0 above v0.9.0
- [ ] Sign and notarise, so the first launch is a double-click on both macOS and
      Windows rather than a right-click Open and a "Run anyway"
- [ ] Tests on every push, not only on a tag

## Missing outright — found by the feature audit

These are not refinements of something half-built; the app cannot do them at
all.

- [x] Clone a repository. Today the only way in is to open a folder that is
      already a repository, so a machine with no checkout has no route at all.
      This is where the profile work pays off: clone from the work GitLab over
      the work key, from personal GitHub over the personal key, without
      choosing either by hand
      — done: `clone_repo` runs `git clone` through the CLI wrapper, so the
      profile's `GIT_SSH_COMMAND` and the machine's credential helper are in
      force; the folder is named after the repository and the destination
      checked before anything is fetched. Reached from the welcome pane
- [x] Create a repository, with a first commit and a `.gitignore`
      — done: `init_repo` runs `git init -b main`, writes a starter
      `.gitignore` and commits it as the profile's identity; with no identity
      anywhere, the `.gitignore` is left untracked in the changes panel and the
      reason is said in the log
- [x] List the repositories a profile's token can see, so cloning is picking
      one from a list rather than pasting a URL. `forge.rs` already
      authenticates and reads pull requests; it has never asked for `/user/repos`
      or `/projects?membership=true`
      — done: `forge_repos` walks both forges' pagination (up to a thousand
      repositories), flattens the fields that matter, and the clone dialog
      offers the list as a searchable picker whenever the profile has a token.
      Works with nothing open — `forge_status` asks only the config — because
      the whole point is choosing a repository before one exists locally
- [x] Manage remotes: add one, rename one, change a URL, remove one. `remotes`
      only lists them, and a repository cloned over HTTPS cannot be moved to ssh
      without dropping to the command line
      — done: the Remote section header has a `+`, and right-clicking a remote
      offers fetch, change address, rename, copy and remove. Remove asks for the
      name typed back and says what stays behind; rename says the tracking
      branches move with the name
- [ ] Check out a pull request's branch, and read its diff. Reviews can be
      listed and opened in a browser, which is where the app stops
- [ ] Compare two branches directly — what is on one and not the other, as a
      diff rather than as two graph rows
- [ ] Show whether a commit is signed. Signing works today because the git CLI
      does it, but nothing in the window says so, and a profile cannot carry a
      signing key next to its SSH key

## Ideas worth doing, not yet started

### For people who do not know git yet

The app is aimed at someone who does not want to learn git before they can use
it. These are the gaps that hurt that person specifically.

- [x] Show the git command that just ran, written the way it would be typed
- [ ] Guided bisect: pick a good commit and a bad one, answer works/broken a few
      times, and be told what broke it. Intimidating on the command line and
      approachable as a wizard, which is the whole argument for this app
- [ ] Blame, and the history of one file — "who changed this line, and why" is
      the question beginners actually ask
- [ ] Search history for a string (`git log -S`): when did this line arrive, and
      when did it go
- [ ] Restore a deleted branch from the reflog. Deleting the wrong branch is a
      beginner's mistake that should cost a click, not an afternoon
- [ ] "Update this branch from main" as a labelled button that explains merge
      against rebase. The drag gesture exists, but it is undiscoverable to
      someone who does not know the operation is possible
- [ ] Stop and ask before committing to `main`, rather than only warning. The
      detection is already there

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
      reveal-in-file-manager path already branches on Windows but is untested.
      Linux now builds and the full test suite passes there, given the GTK and
      WebKit development packages the README names; the window itself has still
      never been looked at on Linux.
- [~] Release build, code signing and notarisation
      — done: pushing a tag drafts a release, builds a universal `.dmg`, an
      NSIS `.exe` and an `.msi`, and an `.AppImage`, `.deb` and `.rpm`, then
      publishes the release once all three runners have finished. Every bundle
      is signed with the updater key, and the app updates itself from Settings →
      Updates: it verifies the signature against the public key compiled into
      the running copy before writing anything, and restarts into the new
      version. Not done: an Apple Developer certificate and notarisation, and a
      Windows code-signing certificate — until those exist, a first launch is a
      right-click Open on macOS and a "Run anyway" on Windows

## How to run it

See the README, which is now the one place the build notes live.

## Verification

`cargo test` runs 150. 149 pass: 59 unit (remote URL parsing, API bases, URL
encoding, AI answer parsing, reasoning levels, one-hunk patch rebuilding, SSH
command building, transport-failure explanations, git command rendering, clone
folder naming, and the config file: its round trip, its migrations and its
corrupt-file path) and 91 integration against real repositories built with the git
CLI — graph lane invariants, divergence reporting, every conflict-resolution
combination, undo and redo, auto-stash, stash operations, cherry-picking several
commits out of order, empty repository, detached HEAD, tracking-branch checkout,
CRLF files, a pull across a divergence, an oversized diff, cloning from a local
remote, creating a repository with a first commit, adding, editing, renaming and
removing a remote against a bare one, and pushing to one — a first push, a
refused one, a force push, and a force push the lease stops — and an
annotated tag, which names an object of its own rather than the commit.

The one failure is on Windows and predates any of this:
`clones_a_repository_into_a_folder_named_after_it` reads a local
`C:Users…` path as a URL and takes everything after the colon as the folder
name, so cloning by pasting a Windows path names the folder after the whole
path. Nothing to do with tags or releases; worth its own fix.

`npm run typecheck` runs and reports 79 errors in seven files, all pre-existing
— this file said 59 for a while, which was never right. `npm run generate` builds the bundle clean.

The UI is checked by rendering the built bundle in a browser and reading the
console; the Tauri window itself cannot be screenshotted from this shell
(Screen Recording permission), so its layout is reviewed by you.
