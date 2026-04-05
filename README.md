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
| / | Search in file |
| : | Command palette |
| ? | Help overlay |
| F8 | Next diff hunk |

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
