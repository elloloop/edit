# edit

A lightweight TUI code editor for agent workflows.

Built with Rust, Ratatui, and tree-sitter. Designed to sit beside Claude Code
in a split terminal — fast to open, fast to navigate, beautiful syntax
highlighting, and first-class diff review.

## Install

```bash
cargo install --path crates/app
```

## Usage

```bash
edit                    # open in current directory
edit src/main.rs        # open a specific file
edit .                  # open directory
edit crates website     # open multiple root folders
```

## Keybindings

| Key | Action |
|-----|--------|
| Ctrl-P | File picker |
| Ctrl-G | Go to line |
| Ctrl-B | Toggle sidebar |
| Ctrl-D | Toggle diff |
| Ctrl-S | Save |
| Ctrl-W | Close tab |
| Ctrl-Q | Quit |
| Ctrl-Left / Ctrl-Right | Move by word |
| Ctrl-Home / Ctrl-End | Jump to start/end of file |
| Ctrl-Z / Ctrl-Shift-Z | Undo / redo |
| / | Search in file |
| : | Command palette |
| ? | Help overlay |
| F8 | Next diff hunk |

Command bar additions:
- `changes` — pick from git-changed files
- `grep <pattern>` — search across the workspace
- `compare <file1> <file2>` — open split compare view
- `edit` — enter edit mode
- `wrap` — toggle word wrap

Benchmarks:
- `edit --benchmark <path>` — render once, print startup time in milliseconds
- `./scripts/benchmark.sh <path>` — compare RSS/startup across `edit`, `edit-gui`, and VS Code

## Architecture

Workspace with independent crates:
- `core-buffer` — ropey text buffer
- `core-diff` — similar-based diff engine
- `core-theme` — VS Code Dark+ theme
- `core-syntax` — tree-sitter highlighting
- `core-picker` — nucleo fuzzy picker
- `core-fs` — file tree + watcher
- `ui-tui` — ratatui rendering
- `app` — main binary
