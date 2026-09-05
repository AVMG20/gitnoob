<div align="center">

<img src="app-icon.png" alt="" width="88" />

# gitnoob

A free, open-source git GUI. Built with Tauri, Rust and Vue.

[Download](https://github.com/AVMG20/gitnoob/releases/latest) ·
[Development](docs/development.md)

<img src="docs/screenshots/main-window.png" alt="The gitnoob window: branches and remotes on the left, the commit graph in the middle, staged and unstaged changes on the right" width="900" />

</div>

## Why this exists

I paid for a git GUI for about five years. It was fine, but I never felt great about renting a tool for something as basic as git, and every time I looked around for a free open-source alternative I came up empty. The ones I found were either abandoned, half-finished, or missing the one feature I used every day.

So I started building my own. This is it. It's the tool I use for my own work now, and it's GPL so it stays free.

## What it does

- Commit graph with real lanes, fast even on very large repos. Squash and rebase merges are drawn too, found by patch-id
- Stage and unstage by file, hunk, or individual lines
- Drag a branch onto another to merge or rebase. Drag a commit onto a branch to cherry-pick
- Interactive rebase by drag and drop: reorder, squash, reword, drop, with a preview before it runs
- Squash a run of commits into one
- Conflict resolution with checkboxes instead of hand-editing markers
- Pull requests and merge requests from GitHub and GitLab, with comments, checks, and a merge button
- Undo and redo for most operations
- Your uncommitted changes follow you across branch switches
- Force push always uses `--force-with-lease` and shows what it would drop first
- Stash, tags, worktrees, remotes, submodules, all in the UI
- Profiles: switch commit identity, forge token, and SSH key together. Tokens live in your OS keychain
- Every action logs the git command it ran. You can also type your own
- Multiple repos open as tabs
- Optional AI commit messages and conflict resolution through OpenRouter. Off unless you add a key

Reads go through libgit2. Writes go through your own `git` binary, so hooks, signing, SSH agent, and credential helpers keep working. Nothing leaves your machine.

## Install

Download from the [latest release](https://github.com/AVMG20/gitnoob/releases/latest). You need `git` installed and on your PATH.

| Platform | File |
|---|---|
| macOS | `.dmg` (Intel and Apple Silicon) |
| Windows | `.exe` or `.msi` |
| Linux | `.AppImage`, `.deb`, or `.rpm` |

The builds are not code-signed yet, so the first launch needs one extra click. On macOS, right-click the app and choose **Open**. On Windows, click **More info** then **Run anyway**. After that, the app updates itself.

Windows gets less testing than macOS and Linux. Bug reports welcome.

## Develop

You need Node and Rust. On Linux, also install the WebKitGTK dev packages listed in [docs/development.md](docs/development.md).

```sh
npm install
npm run app          # dev mode: Nuxt dev server plus the Tauri window
```

Run tests and type checks:

```sh
npm test
npm run typecheck
cargo test --manifest-path src-tauri/Cargo.toml
```

## Build

```sh
npm run app:build
```

The bundle lands in `src-tauri/target/release/bundle`.

More on the architecture, testing, and releasing in [docs/development.md](docs/development.md).

## Shortcuts

`⌘⇧F` fetch · `⌘⇧L` pull · `⌘⇧P` push · `⌘Enter` commit · `⌘B` branch ·
`⌘⇧S` stash · `⌘F` search · `⌘P` switch repo · `⌘Z` undo · `⌘,` settings

Use `Ctrl` on Windows and Linux. Full list under Settings → Shortcuts.

## License

[GPL-3.0-only](LICENSE).
