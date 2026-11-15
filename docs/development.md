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

## The graph

`graph.rs` walks every ref newest first and hands each commit a lane. The lane table holds, per column, the commit that column is waiting to reach; a commit claims the column waiting for it, hands it on to its first parent, and opens a column for every other parent. Each row's line segments are derived from the table just before and after that step, so a row draws on its own and the list can be virtualized.

A few decisions that are easy to undo by accident:

- **The trunk owns the leftmost column.** `main`, `master`, or whatever `gitnoob.trunk` names, the same answer the sidebar measures branches against. Anchored on HEAD instead, the whole picture slid sideways every time you checked something out.
- **A trunk behind its upstream is anchored at the upstream's tip.** Otherwise the commits still to pull were the one line that could not have the column, and were drawn as a branch beside you rejoining at your commit — a merge, for what is a fast-forward. They sit straight above you, held back (`Segment.faint`, `GraphRow.unpulled`), the mirror of the hollow ring on a commit not pushed yet.
- **The line you are on is marked** from HEAD's commit down to where it runs into another line (`Segment.current`), and the frontend draws it a shade brighter. Not above HEAD: that is the rest of the branch, or the upstream's commits, and neither is where you are.
- **Squash and rebase merges are found by patch-id.** Git keeps no record of where a squash commit came from, so a branch merged that way dangles. Every branch tip nothing is built on has the patch from its fork point to its tip measured, and the single-parent commits above it are asked whether they make the same patch; a rebase is the same question one commit at a time. A match is drawn as a broken line from the commit that carries the work down to the branch tip (`GraphRow.carries`). Only the page is looked at, and only against the trunk's fork point: a wrong link is worse than a missing one. Patch-ids are cached for the life of the process, since a commit's content never changes.

**Soloing** draws only some branches (`graph::soloed`, `store.solo`). The walk starts from those refs alone, a local branch bringing its upstream along, and the first one named takes the leftmost column unless the trunk is among them. `commit_depth` takes the same list, since it has to count the same rows. The solo is kept per repository for the session, not across restarts, and a soloed branch that disappears is dropped on the next refresh.

A new branch is named in the graph itself, on the row it starts from (`useBranchNaming`). The dialog stays for when the graph is not on screen.

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
  recover.rs    discard backups, the reflog view, dropped stashes
  home.rs       the home tab
  config.rs     settings, profiles, projects
  identity.rs   the name and address a repository commits with
  forge/        GitHub and GitLab
  review.rs     one pull or merge request, rolled up
  ai.rs         OpenRouter
  ssh.rs        per-profile keys
  sign.rs       commit signing
  avatar.rs     author pictures, cached
  watch.rs      filesystem watcher
  files.rs      the Files tree and the file finder: listing, fuzzy matching, the path guard
  search.rs     search inside files, streamed back as events
  edit.rs       reading a file for the editor and saving it
  symbols.rs    code navigation and unused code, with tree-sitter
  mouse.rs      the mouse's back and forward buttons on macOS
src-tauri/tests
  repo.rs       integration tests against real repos built with git
  forge.rs      which forge project and profile, the home tab, after a merge
app
  app.vue           shell, tabs, repo picker
  composables/      useGit.ts is the shared store; useInvoke.ts wraps every call
  components/       everything visual
  assets/css/       main.css, file-icons.css and the generated themes.css
test                vitest, unit and happy-dom component tests
scripts
  release.mjs       cuts a release
  theme/            palette and the generator for themes.css
```

## Tabs and history

The middle of the window is the graph or a file, with a strip of tabs over it once a file is open. The graph is always the first tab and can't be closed; in `useTabs.ts` it is `active === null` rather than an entry in the list. Each project tab has its own set of file tabs and its own history for the session, swapped in by `openRepo`.

`store.viewer` is not a field any more. It is a getter worked out from the active tab, in the shape it always had (`{ path, side }` or `{ path, commit }`), so `DiffViewer`, staging and discarding read it as before. Setting it still works and is kept for the places that walk from one file to the next: a file retargets the tab on screen (or opens one from the graph), and `null` closes it. New code calls `openTab`, `retarget`, `closeTab` and `showGraph` instead.

A few rules that are easy to break:

- **A single click previews.** A preview tab is replaced by the next preview, so reading down a list of files doesn't pile up tabs. A double click, a jump or a search result opens a tab that stays.
- **What goes into history:** a tab switch, showing the graph, opening a file from the tree, closing the tab on screen (so back reopens it), and later jumps and search results. Not a preview replacing a preview, and not walking files with the arrows or after a stage, which retarget the tab in place. A new place clears forward. Both stacks stop at 100.
- **The Diff | File switch belongs to the tab.** `diffMode.mode` is kept in step with the active tab both ways, so a tab comes back in the mode it was left in.
- **Clicking a branch, a tag or a stash shows the graph.** `revealCommit` and `selectStash` call `showGraph`; the file tabs stay.

Back and forward are ⌘[ / ⌘] (Ctrl elsewhere, plus Alt+← / Alt+→ off macOS), the arrows in the tab strip, and the mouse's side buttons. The page listens for buttons 3 and 4 on `mouseup` and prevents the default on `mousedown`, `mouseup` and `auxclick`, because WebView2 would otherwise run its own history navigation and leave a blank window. WKWebView on macOS doesn't reliably hand those buttons to the page at all, so `mouse.rs` installs an `NSEvent` local monitor for `otherMouseDown` and emits `nav-button`. When a press arrives both ways, `navFrom` in `useNavButtons.ts` drops the second copy.

`chordOf` in `useShortcuts.ts` reads the physical key (`event.code`) when Alt is held and `event.key` isn't a plain letter or digit, because Option on a Mac types a different character (⌥F is `ƒ`) and no ⌘⌥-letter chord could match otherwise. On a Mac, Control is spelled `ctrl` so it never matches `mod`; elsewhere `ctrl+` in the list means the same as `mod+`.

## The Files tree

Every list of files draws a type icon before the name (`FileIcon.vue`, mapping in `fileIcons.ts`): whole names first (package.json, Dockerfile, .env*, README*…), then `.blade.php`, then the extension, cached by name so a long list asks once per name. The colours are in `assets/css/file-icons.css`, which is hand-written: each language colour is mixed with `--fg` (`color-mix` in oklab) so pale ones like JavaScript yellow still read on a light theme. The status colour stays on the name. Settings → File icons turns them off everywhere.

`files.rs` has two commands. `list_dir` reads one folder; the tree only ever reads folders somebody opened, so a big repository shows straight away. `find_files` fuzzy-matches a query against every path with `nucleo-matcher`, from a list walked once with the `ignore` crate and kept until the watcher sees a file appear, go, or a `.gitignore` change (or a minute passes, for filesystems the watcher can't see). A newer request makes an older one give up.

What git ignores is decided by the `ignore` crate: `.gitignore` at every level, `.git/info/exclude` and the global excludes. A folder that is itself ignored is checked with libgit2's `is_path_ignored` first, because a pattern like `vendor/` says nothing about `vendor/x/y.php` when entries are matched one at a time. Ignored entries are listed and flagged, never hidden, and never read until their folder is opened.

Every path from the window goes through `files::resolve` before anything touches the disk: joined to the work tree, canonicalized, and refused if it ends up outside the work tree or inside a `.git`, symlinks included. Any new command that takes a path should use it too.

The tree's colours come from `store.status`, not from walking: a changed file in its status colour with a dot (filled staged, ring unstaged, half both), and every folder above it in the same colour. Deleted files are added back into their folder from the status, struck through. Which list the sidebar shows, the filter, the open folders and the scroll are kept per project in `useFileTree.ts`.

## The search window

One window, three modes: inside files (⌘⇧F), file names (⌘⇧N, or ⌘E for the recent files) and symbols (⌘⌥O). Another mode's key or Tab switches mode and takes the text along. Fetch moved from ⌘⇧F to ⌘⌥F to make room. It is an `AppModal` with `palette` set: no title row, near the top, most of the window's height. The box, the mode switch and, for text, the option toggles share one row; ⌥C, ⌥W and ⌥R flip case, word and regex by `event.code`, because on a Mac ⌥ types ç, ∑ and ®. The list and preview split is a `ResizeHandle` on the `search` edge in `usePanes.ts`. ← and → fold a file's hits under its heading, ⌘C copies path:line, and a click on a line in the preview opens it there. The preview marks every hit in the file and wraps its lines.

Hit lines in the list are coloured in their file's language (`snippetColours.ts`): each file's hit lines are parsed together once with the editor's Lezer grammar and painted with the same theme specs as inline styles, cached by file and text, plain until the grammar has loaded. A file opened from the window, a jump, the usages popup, or back and forward passes `reveal: 'switch'` to `openTab`, which brings the sidebar to the Files tree with the file revealed; opens from the right panel, a commit or a review leave Branches showing (the tree still follows the active tab whenever it is on screen).

The file name mode adds two shaped matches on top of the fuzzy one in `files.rs`: a query with a slash matches the starts of consecutive folders (`exact/make`), and capitals match the capitals of a name (`MECC` for MakeExactConnectionCommand). Either ranks above any fuzzy match. Symbols mode is `find_symbols` over the M5 symbol index: classes, interfaces, functions, methods, modules, types and macros, fuzzy on the name with the same capitals rule, and it opens at the definition.

`search_text` in `search.rs` walks the same cached path list as the finder, with `grep-searcher` and `grep-regex`, on a thread of its own. It never answers the call with results: it sends `search:hit` (one file's lines) and `search:done` events, each tagged with the request number the window sent. A newer request makes the running one stop at the next file, and the window drops any event that isn't for the request it is waiting on. Typing waits 120 ms before a search goes out. Results stop at 2,000 lines; the rest are only counted and shown as "N more". Match ranges are in UTF-16 units, because that is what JavaScript indexes strings in. "Changed files only" searches what `store.status` would list, plus conflicted files.

The file name mode asks `find_files` with the recent and changed paths as `also`, so they come back whatever their score, and `findNames` in `useSearch.ts` ranks them recent, changed, then the rest. `app/Tax.php:42` opens at line 42.

## The editor

File mode on a working-tree tab is a CodeMirror 6 editor (`FileEditor.vue`, extensions in `useCodeMirror.ts`). A commit's copy and a file that is gone keep the read-only `FileView`. Language packages are imported only when a file of that kind is opened. Syntax colours come from the active Shiki theme's rules, so the editor and the diff agree; the chrome comes from the CSS variables.

Saving is plain file I/O in `edit.rs`, not a git command. `read_file` hands back the text with a sha256 of the bytes on disk, the line ending, and whether there was a BOM. `write_file` takes the hash the editor last saw and refuses with an error starting `Conflict` if the file no longer has it, so nothing written by another program in the meantime is lost. The write goes to a temp file next to the original and is renamed over it, keeping the permissions. CRLF is put back if the file was CRLF, the BOM is put back, and a missing final newline stays missing, byte for byte. A file that mixes CRLF and LF, or has a lone `\r`, opens read-only (`endings`): the editor only knows `\n`, so there is no way to write it back without changing lines nobody touched, and `write_file` refuses it too. A file deleted under the editor and kept is written again with the endings and BOM it had. A binary, LFS-pointer, non-UTF-8 or over-2 MB file opens read-only with a note saying why. Every path goes through `files::resolve`.

The bar over an open file is decided by one pure function, `headerState` in `fileHeader.ts`, from the file (read-only reason, ignored, missing), its status entries and the tab. A tracked file with no changes gets no counts, no Stage or Discard and no Diff to switch to (Tab and ⌘D do nothing); it gets History, Blame, Find, Reveal in Files and a ⋯ menu instead. Stage and Unstage follow the status: all staged is Unstage only, partly staged is both. An untracked file says "new" and has Add and Delete (the right panel's question asks first); an ignored one says "ignored", asked once with `path_ignored`, and has no git buttons. Saving and saved are not said in the bar (the tab's dot and the panel's M say it); only a failed save ("Not saved", the error in its tooltip) and "Changed on disk" show, as a warning chip. The path is a breadcrumb whose folders reveal in the Files tree; in File mode the bar also shows Ln:Col (click to go to a line), LF or CRLF, UTF-8 or UTF-8 BOM, and the language. Under 760 px wide the quieter tools fold into ⋯. ⌘S outside the editor only stages a file the status lists.

Indentation is worked out per file in `indentation.ts` and set as CodeMirror's `indentUnit` and `tabSize` (left at CodeMirror's default of two, a four-space PHP file got six spaces on a new line inside a method). First a per-file override picked from the header's "Spaces: 4" / "Tabs" chip, then `.editorconfig` (`editorconfig.rs`: `indent_style`, `indent_size`, `tab_width`, globs, `root = true`, read from the file's folder up to the work tree through `files::resolve`), then the file's own leading whitespace (the commonest step between indented lines), then Settings → Editor when it says Spaces or Tabs, then the language's custom. Tab and ⇧Tab indent and dedent by the unit in the editor.

When it saves: 800 ms after the last keystroke, when the editor loses focus, when the tab is left, on ⌘S, and when the window closes. One save runs at a time; the one a tab leaves behind waits for the save under way and starts from the hash it left. Edits whose save was refused are kept in `drafts` and come back with the tab. Drafts and save states are kept per repository and tab (`editKey`), and every read and write names its repository (`invokeAt`), so a save that lands after a project switch still goes to the right one.

Nothing unsaved goes quietly. Closing a tab whose edits are in conflict or failed to save asks first (`closeTabsSafely`). Closing the window, ⌘Q, the Dock's Quit, logout and installing an update all go through `prepareToQuit` (`useQuit.ts`, `quit.rs`): everything is saved, and if anything cannot be, it asks. On macOS the menu bar is built in `lib.rs` so Quit is ours, and `applicationShouldTerminate:` is added to tao's app delegate for the Dock, logout and shutdown. The backend only hands a quit to the page while the page has said it is listening (`quit_ready`) and holds edits not on disk (`quit_guard`); with nothing unsaved, or a blank window, the app just quits and a logout goes on. A page that does not acknowledge within ten seconds is taken to be gone, so nothing can keep the app from quitting.

When the watcher reports a change, the editor reads the file again and compares hashes. Its own save has the hash it already knows, so it is ignored. A change with nothing typed is taken quietly, caret and scroll kept. A change while there are unsaved edits shows "Changed on disk" with Keep mine, Take theirs and Compare; Compare is the conflict resolver's grid (`CompareDialog.vue`) over your text and the disk's.

The change bars in the gutter come from `head_file_diff`, the file against the last commit with staged and unstaged changes together. Clicking one opens a card under that change (`ChangeCard.vue`, the logic in `changeCard.ts`) with the lines as they were, in the editor's colours on the diff's red, the changed words picked out. From the card: Revert puts the old lines back as one editor transaction (so ⌘Z and the autosave handle it, and no git command runs), Stage this change finds the same lines in the unstaged diff by their line numbers and goes through `apply_hunk`, Copy old text, and Show in Diff, which is what a click on a bar used to do. The card reads against HEAD like the bars, so on a partly staged file it shows what was there at the last commit, and Stage only moves what is still unstaged in those lines. It closes on Esc, a click outside, or when its change is gone from the next read of the hunks; ⌥↓ / ⌥↑ move it while it is open. A read-only file gets the card without Revert and Stage. ⌥↓ / ⌥↑ step between changes, ⌘G goes to a line, ⌘F is the existing find bar. The editor's own keymap leaves out move-line (⌥↑/↓) and ⌘[ / ⌘], which are back and forward. Shortcuts marked `editor: true` in `useShortcuts.ts` still fire with the caret in the editor; every other one is left to the editor, so ⌘Z there is the editor's undo, not git's.

## Images

An image file (png, jpg, gif, webp, avif, bmp, ico, svg) is drawn, not read as text. `image_read` in `image.rs` returns its bytes, base64, from the work tree (through `files::resolve`) or from a blob in the index, HEAD, a commit or that commit's parent (libgit2), with a type sniffed from the bytes; the extension only decides whether to ask. Anything over 30 MB is measured and not sent, and an LFS pointer comes back as one, so the viewer shows the existing LFS page. The window turns the bytes into a Blob URL, revoked when the view goes, and only ever draws it through `<img>`, so an SVG's scripts never run; that is why the CSP's `img-src` has `blob:`. File mode is `ImageView.vue` (fit, 100%, zoom by button, ⌘+/⌘−/⌘0, wheel or pinch, drag to pan, the size and type in the header), read again when the watcher sees the work tree change. Diff mode is `ImageDiff.vue`, before and after side by side or as an onion skin: the index against the disk on the unstaged side, HEAD against the index on the staged one, the parent against the commit on a commit tab. An SVG has a Source switch that opens its text in the editor. The search window's preview shows an image small and fitted.

## Code navigation and unused code

`symbols.rs` parses every file in PHP, TypeScript/JavaScript (and the `<script>` blocks of `.vue` files), Rust, Python and Go with tree-sitter, and runs each grammar's own tags query over it to get definitions and references. The tags query goes by name. For PHP and TypeScript, `receivers.rs` walks the same tree once more for what the receiver of each method call is: `$this`, `self::`, `static::` and `parent::`, `Foo::m()`, `(new Foo)->m()`, a variable assigned `new Foo`, `app(Foo::class)`, `resolve(Foo::class)` or `Foo::make()`, a parameter, promoted or typed property, or `@var` docblock; in TypeScript `this.`, `super.`, `Foo.`, `new Foo()`, and typed parameters, variables and fields. `use` and `import` aliases are undone, and what each class extends, implements or uses as a trait is kept, so the receiver is stored on each reference at index time. A method's usages are then definite (the receiver is its class, or one above or below it), possible (receiver unknown, such as an untyped `$x->handle()`), or not its usages at all (another class). Rust, Python and Go stay by name. A reference also has a shape (`foo()`, `$x->foo()`, `Foo::foo()`, a type), so a method call looks for methods and a function call for functions. The grammars are C compiled by `cc`, so nothing has to be installed on any platform. Keep `tree-sitter` and the grammar crates on versions that share `tree-sitter-language`.

The index is in memory, one per work tree. `open_repo` starts building it on a thread of its own, the watcher hands it the files that changed, and anything ignored or over 1 MB is left out. So are dependencies and build output even when git does not ignore them (`symbols::generated`: `vendor/`, `node_modules/`, `dist/`, `.output/`, `.nuxt/`, a top-level `storage/` and `storage/framework/`, `bootstrap/cache/`, `public/build/`). A command that needs it before it is built waits for it.

What the framework calls is counted by how its class is used, not by its name: `handle` on a command, job, listener or middleware, `__invoke`, `__construct` and lifecycle hooks on a class count `Foo::class`, `new Foo`, `Foo::dispatch(`, type hints and calls on the class from elsewhere; an artisan command also counts its `$signature` name in `Artisan::call`, `$this->call`, `$this->artisan` and `->command(...)` strings. A controller action counts the uses of its controller and the routes in the `[Controller::class, 'action']` form that name it. A `#[tauri::command]` counts the front end's `invoke('name')` calls. The lens reads "2 uses of ExportAnalyticsCommand", "called by the framework · 2 references", or with none, "called by the framework".

The commands: `symbol_at(path, offset, text)` says what name is at a UTF-16 offset, in the editor's text when it is ahead of the disk. `definitions(name, path, shape)` lists places in files of the same language, and `usages(name, path, kind, line)` lists a definition's usages split into `definite` and `possible`. `unused_in(path, text)` says what to gray out and `usage_counts(path, text)` puts "3 usages" over definitions, with possible ones muted after it ("2 usages · 50 possible"; Settings → Behaviour turns those off). ⌘-click and ⌘J go through `jumpFrom` in `useSymbols.ts`: on a reference to the definition, on a definition to the usages (a notice for none, a jump for one definite, otherwise the popup: definite first, changed files first, then a folded "Possible (n)" group). Graying out does not use the split: any reference by name keeps a definition alive.

Graying out is careful on purpose, because a wrong gray tells someone to delete code that runs:

- A function or method is only marked when its name appears nowhere else in the repository's text. The last check is `search::count_word`, the text search over every file git does not ignore, in any language, strings included.
- Only private functions and methods, and local variables, are dimmed. Public and exported ones get a dotted underline, since something outside the repository may call them.
- Entry points are never marked: constructors and magic methods, controller actions, `handle()` and the framework's other hooks, protected methods, public methods of a class that extends or implements something, Vue lifecycle hooks, `main`, anything with an attribute such as `#[test]` or `#[tauri::command]`, trait implementations, and every file under a tests folder.
- A local variable is dimmed when its name appears once in the function that declares it. A function that can read variables by name (`compact`, `extract`, `$$`, `eval`, an `include`) is left alone.
- Unused code on lines added since the branch left main, or not committed yet, gets a "new · 0 usages" badge (`diff::branch_added_lines`).

`cargo test --test repo -- --ignored own_source --nocapture` prints what would be grayed out in this repository's own source.

Commit tabs still use the read-only `FileView`, not CodeMirror, so ⌘-click and the counts are only in working-tree files.

### Keys that moved in 0.10

Fetch is ⌘⌥F (was ⌘⇧F, now search inside files). Next and previous changed file are ⌘⌥↓ / ⌘⌥↑ (were ⌘] / ⌘[, now back and forward). Next match in the find bar is ↵ and ⇧↵ in the box (⌘G goes to a line now). A key someone rebound keeps their binding. The release notes are written by hand on the release page, so these have to be copied there.

## Undo

Each work tree has its own history, keyed by its canonical path and written to `gitnoob-journal.json` in that work tree's git directory, so it survives a restart and two worktrees of one repository never share steps. When the history is read, every step is checked against the repository as if the newer steps had been taken; one that no longer fits is shown greyed out and can be forgotten, never attempted.

A few rules that are easy to break:

- **Record after the operation, never before**, with `journal::record` (branch moves) or `journal::record_entry` (anything carrying more). A step that stops on conflicts calls `journal::begin_pending`, and whatever finishes it reads `journal::pending` first and calls `journal::finish_pending` after. A commit that concludes a merge is recorded as a merge, never as a soft commit.
- **Anything that throws work away saves it first.** Discards and deleted untracked files go through `recover::look(.., true)` (raw bytes, `hash-object -w --no-filters`) before and after, and the difference is the step. Undo refuses if the file changed since.
- **A branch that is not checked out is moved with `update-ref` and an old value**, so git itself refuses if it moved in between. Undo and redo refuse while a merge, rebase, cherry-pick or revert is running.
- **A stash apply or pop saves what its paths held first.** `work::before_stash` reads each touched path's index entry and file before it goes on, and the undo puts exactly that back, so work staged before the pop survives it.
- **Branches are deleted by commit.** A tidy passes the oid it showed and deletes with `update-ref -d <ref> <oid>`; the whole batch is one `Branch` step, and the notice's Undo calls `undo_step` with that step's id rather than putting the branches back by itself.
- **The file is shared with other windows.** `journal_at` re-reads it before every use and the file wins for steps both have seen. A file this version cannot parse is moved aside to `gitnoob-journal.json.bad-<time>`, never written over, and one that cannot be read at the moment (locked by another program, a permission gone wrong) is left alone until it can. A full history lets go of other steps before a discard's, but never of the step just recorded.

Dropped stashes and saved files are unreachable objects, so they are only as safe as git's garbage collection allows (two weeks by default).

## Themes

`app/assets/css/themes.css` and `app/composables/themeList.ts` are generated from `scripts/theme/palette.mjs` by `npm run theme`. Edit the palette, not the output. A test regenerates both and fails if the checked-in files drifted, and checks every theme against WCAG contrast ratios.

The default is Porcelain by day and Graphite by night: "Match system" in Settings → Appearance follows the OS between the two while the window is open (`useTheme.ts`). They have ladders of their own in the palette, with slightly crisper body text (`text`) and softer lines (`lines`) than the rest.

The window is built like a workbench. Panes meet edge to edge, and the resize handles are the one-pixel hairlines between them. The chrome round the work is `--canvas`: the toolbar, the sidebar and the inspector (`app.vue` paints every `aside` in the body with it). The work itself is `--bg`: the graph, a diff, a review (every `section`). The tab strip is `--deep`, one step further, so the open tab can run into the toolbar below it. A component that sits in that grid should not paint its own background.

## Content security policy

`app.security.csp` in `tauri.conf.json` is `default-src 'self'`, with `img-src` opened to `data:` and `https:` for avatars and screenshots in PR descriptions and to `blob:` for the image viewer, and `connect-src` opened to `ipc: http://ipc.localhost` so calls can reach the backend at all.

Nuxt's inline import map has to stay inline. `tauri-build` hashes the inline scripts at build time and adds the hashes to the policy, which is how they are allowed without `'unsafe-inline'`.

CodeMirror needs `style-src 'unsafe-inline'`, which the policy has: it injects `<style>` elements and sets inline styles. It uses no workers (`worker-src 'none'` stays) and no `eval`.

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

### Code signing

The updater key above is not code signing. It proves an update came from this project to a copy that is already installed. What stops macOS and Windows warning about the first install is a certificate, and that is separate and optional: each platform signs when its secrets are on the repository and builds unsigned when none of them are. Setting some but not all of them fails the build, on purpose.

**macOS.** This needs the Apple Developer Program, which is $99 a year. With it, the app is signed with a Developer ID and notarized, and opens with the ordinary "downloaded from the internet" prompt instead of being called damaged.

1. Join at developer.apple.com/programs. An individual account is fine; the certificate then carries your own name.
2. In Xcode → Settings → Accounts → Manage Certificates, add a **Developer ID Application** certificate. Or make one at developer.apple.com → Certificates.
3. In Keychain Access, find it under My Certificates, right-click → Export, save as `.p12` with a password.
4. At account.apple.com → Sign-In and Security → App-Specific Passwords, make one for notarization.
5. Your team id is on developer.apple.com → Account → Membership details.

```sh
base64 -i DeveloperID.p12 | gh secret set APPLE_CERTIFICATE
gh secret set APPLE_CERTIFICATE_PASSWORD   # the .p12 password
gh secret set APPLE_ID                     # the account email
gh secret set APPLE_PASSWORD               # the app-specific password
gh secret set APPLE_TEAM_ID
```

The bundler imports the certificate into a throwaway keychain, signs with the hardened runtime, sends the app to Apple and staples the ticket. Notarization adds a few minutes to the macOS job.

**Windows.** This uses Azure Artifact Signing (it was called Trusted Signing), about $10 a month. It is the cheapest way to sign from CI: Microsoft holds the key, so there is no hardware token to plug into a runner. Identity validation is open to organisations, and to individuals in a limited set of countries; check the current list before paying for anything.

1. In the Azure portal, create an Artifact Signing account, then a Public Trust identity validation, then a certificate profile.
2. Register an app in Entra ID, give it a client secret, and give it the *Artifact Signing Certificate Profile Signer* role on the account.

```sh
gh secret set AZURE_TENANT_ID
gh secret set AZURE_CLIENT_ID
gh secret set AZURE_CLIENT_SECRET
gh secret set AZURE_SIGNING_ENDPOINT    # e.g. https://weu.codesigning.azure.net
gh secret set AZURE_SIGNING_ACCOUNT
gh secret set AZURE_SIGNING_PROFILE
```

The workflow installs `artifact-signing-cli` and passes it to Tauri as `bundle.windows.signCommand` in a config that only that build sees, so local builds do not need it.

A signature does not make SmartScreen go quiet straight away. Since 2024 no certificate, EV included, does that. It shows the publisher's name and lets reputation build up as people run it, and until it has, a new release can still get the "unrecognised app" screen.

### Updating in place

Settings → Updates shows the installed version, a check button, and the release notes of whatever is on offer. Installing downloads the bundle, verifies the signature against the compiled-in public key, writes it and restarts. On Linux this works from the AppImage; `.deb` and `.rpm` installs update through the package manager.

The app also checks once at launch, quietly. A new version shows as a line in the profile menu and a dot next to Updates in settings. "Look for a new version at launch" in that page turns it off.

## Screenshots

The README's images live in [`screenshots/`](screenshots/).
