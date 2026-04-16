use crate::terminal_input::translate_key_event;
use crate::state::{ActivePicker, AppMode, AppState};
use crate::workspace::{FocusTarget, SplitAxis};
use core_buffer::Direction;
use core_terminal::TerminalLauncher;
use core_picker::{file_picker, SearchMatch};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::process::Command;

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match state.mode {
        AppMode::Help => return handle_help_key(state, key),
        AppMode::Picker => return handle_picker_key(state, key),
        AppMode::Normal => {}
    }

    if state.editing {
        return handle_edit_key(state, key);
    }

    if matches!(state.focus_target, FocusTarget::TerminalPane(_)) {
        return handle_terminal_workspace_key(state, key);
    }

    // Normal mode: check sidebar focus
    if state.sidebar_focused && state.sidebar_visible {
        if handle_sidebar_key(state, key)? {
            return Ok(());
        }
    }

    handle_normal_key(state, key)
}

fn handle_normal_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match (key.modifiers, key.code) {
        // === Ctrl shortcuts (power user) ===

        // Quit
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            state.quit = true;
        }

        (mods, KeyCode::Char('Z')) if mods.contains(KeyModifiers::CONTROL) => {
            if state.current_buffer_mut().redo() {
                state.set_status("Redo");
            }
        }
        (mods, KeyCode::Char('z')) if mods == KeyModifiers::CONTROL => {
            if state.current_buffer_mut().undo() {
                state.set_status("Undo");
            }
        }

        // Save
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
            let result = state.current_buffer_mut().save();
            match result {
                Ok(()) => {
                    state.clear_external_conflict_for_current_buffer();
                    state.set_status("Saved");
                }
                Err(e) => state.set_status(&format!("Save failed: {e}")),
            }
        }

        // Toggle sidebar
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            state.sidebar_visible = !state.sidebar_visible;
        }

        // Toggle focus between terminal and editor workspaces
        (KeyModifiers::NONE, KeyCode::F(6)) | (KeyModifiers::CONTROL, KeyCode::Char('`')) => {
            state.toggle_workspace_focus();
            state.set_status(if state.terminal_workspace_focused() {
                "Terminal workspace focused"
            } else {
                "Editor workspace focused"
            });
        }

        // File picker
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            let picker = file_picker(&state.root_dir);
            state.picker = Some(ActivePicker::File(picker));
            state.mode = AppMode::Picker;
        }

        // Go to line (shortcut: prompts via command bar)
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            state.command_input = ":".to_string();
        }

        // Toggle diff
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            state.diff_mode = !state.diff_mode;
            if state.diff_mode {
                state.compute_diff_for_current();
                state.set_status("Diff view on");
            } else {
                state.set_status("Diff view off");
            }
        }

        // Close tab
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            close_or_confirm_tab(state);
        }

        // Next tab
        (KeyModifiers::CONTROL, KeyCode::Tab) | (KeyModifiers::NONE, KeyCode::F(2)) => {
            if !state.buffers.is_empty() {
                state.active_buffer = (state.active_buffer + 1) % state.buffers.len();
                state.close_dirty_confirm = false;
            }
        }

        // Previous tab
        (KeyModifiers::CONTROL | KeyModifiers::SHIFT, KeyCode::BackTab)
        | (KeyModifiers::NONE, KeyCode::F(1)) => {
            if !state.buffers.is_empty() {
                state.active_buffer = if state.active_buffer == 0 {
                    state.buffers.len() - 1
                } else {
                    state.active_buffer - 1
                };
                state.close_dirty_confirm = false;
            }
        }

        // Next diff hunk
        (KeyModifiers::NONE, KeyCode::F(8)) => {
            if state.diff_mode {
                let buf = &state.buffers[state.active_buffer];
                let path_clone = buf.path.clone();
                let current_line = buf.cursor_line + 1;
                if let Some(ref path) = path_clone {
                    let target_info = state.diffs.get(path).and_then(|diff| {
                        diff.hunks
                            .iter()
                            .find(|h| h.new_start > current_line)
                            .map(|h| (h.new_start.saturating_sub(1), h.new_start))
                    });
                    if let Some((target, hunk_line)) = target_info {
                        state.current_buffer_mut().go_to_line(target);
                        state.set_status(&format!("Hunk at line {hunk_line}"));
                    }
                }
            }
        }

        // Next search result
        (KeyModifiers::NONE, KeyCode::F(3)) => {
            if !state.last_search.is_empty() {
                let query = state.last_search.clone();
                search_forward(state, &query);
            }
        }

        // Previous search result
        (KeyModifiers::SHIFT, KeyCode::F(3)) => {
            if !state.last_search.is_empty() {
                let query = state.last_search.clone();
                search_backward(state, &query);
            }
        }

        // === Navigation keys → editor ===
        (KeyModifiers::NONE, KeyCode::Up) => {
            state.current_buffer_mut().move_cursor(Direction::Up, 1);
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            state.current_buffer_mut().move_cursor(Direction::Down, 1);
        }
        (KeyModifiers::NONE, KeyCode::Left) => {
            state.close_dirty_confirm = false;
            state.current_buffer_mut().move_cursor(Direction::Left, 1);
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            state.close_dirty_confirm = false;
            state.current_buffer_mut().move_cursor(Direction::Right, 1);
        }
        (KeyModifiers::CONTROL, KeyCode::Left) => {
            state.close_dirty_confirm = false;
            state
                .current_buffer_mut()
                .move_cursor(Direction::WordLeft, 1);
        }
        (KeyModifiers::CONTROL, KeyCode::Right) => {
            state.close_dirty_confirm = false;
            state
                .current_buffer_mut()
                .move_cursor(Direction::WordRight, 1);
        }
        (KeyModifiers::NONE, KeyCode::Home) => {
            state.current_buffer_mut().move_cursor(Direction::Home, 1);
        }
        (KeyModifiers::NONE, KeyCode::End) => {
            state.current_buffer_mut().move_cursor(Direction::End, 1);
        }
        (KeyModifiers::CONTROL, KeyCode::Home) => {
            state
                .current_buffer_mut()
                .move_cursor(Direction::FileStart, 1);
            state.set_status("Top of file");
        }
        (KeyModifiers::CONTROL, KeyCode::End) => {
            state
                .current_buffer_mut()
                .move_cursor(Direction::FileEnd, 1);
            state.set_status("Bottom of file");
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            let page = state.viewport_height.max(1);
            state
                .current_buffer_mut()
                .move_cursor(Direction::PageUp, page);
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            let page = state.viewport_height.max(1);
            state
                .current_buffer_mut()
                .move_cursor(Direction::PageDown, page);
        }

        // Tab → sidebar focus
        (KeyModifiers::NONE, KeyCode::Tab) => {
            if state.split_buffers.is_some() {
                state.toggle_split_focus();
            } else if state.sidebar_visible {
                state.focus_sidebar();
            }
        }

        // === Command bar input ===

        // Enter → execute command
        (KeyModifiers::NONE, KeyCode::Enter) => {
            execute_command_input(state)?;
        }

        // Esc → clear command input
        (KeyModifiers::NONE, KeyCode::Esc) => {
            if !state.command_input.is_empty() {
                state.command_input.clear();
            } else if state.split_buffers.is_some() {
                state.exit_split();
                state.set_status("Exited compare view");
            }
            state.close_dirty_confirm = false;
        }

        // Backspace → delete from command input
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            state.command_input.pop();
        }

        // All printable characters → command input
        (KeyModifiers::NONE, KeyCode::Char(ch)) | (KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
            state.command_input.push(ch);
        }

        _ => {}
    }

    Ok(())
}

fn handle_terminal_workspace_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            state.quit = true;
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(6))
        | (KeyModifiers::CONTROL, KeyCode::Char('`')) => {
            state.focus_editor();
            state.set_status("Editor workspace focused");
            return Ok(());
        }
        (KeyModifiers::CONTROL, KeyCode::Tab) => {
            state.cycle_terminal_focus(true);
            state.set_status("Terminal pane cycled");
            return Ok(());
        }
        (mods, KeyCode::BackTab) if mods.contains(KeyModifiers::CONTROL) => {
            state.cycle_terminal_focus(false);
            state.set_status("Terminal pane cycled");
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(7)) => {
            state.relaunch_active_terminal(TerminalLauncher::Shell);
            state.set_status("Launched shell");
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(8)) => {
            state.relaunch_active_terminal(TerminalLauncher::Claude);
            state.set_status("Launched Claude");
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(9)) => {
            state.relaunch_active_terminal(TerminalLauncher::Goose);
            state.set_status("Launched Goose");
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(10)) => {
            let pane_id = state.split_active_terminal(SplitAxis::Vertical)?;
            state.set_status(&format!("Split terminal vertically into pane {pane_id}"));
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(11)) => {
            let pane_id = state.split_active_terminal(SplitAxis::Horizontal)?;
            state.set_status(&format!("Split terminal horizontally into pane {pane_id}"));
            return Ok(());
        }
        (KeyModifiers::NONE, KeyCode::F(12)) => {
            if state.close_active_terminal() {
                state.set_status("Closed terminal pane");
            } else {
                state.set_status("Cannot close the last terminal pane");
            }
            return Ok(());
        }
        _ => {}
    }

    if let Some(bytes) = translate_key_event(key, state.active_terminal_application_cursor()) {
        if let Err(error) = state.send_input_to_active_terminal(&bytes) {
            state.set_status(&format!("Terminal input failed: {error}"));
        }
    }

    Ok(())
}

fn execute_command_input(state: &mut AppState) -> anyhow::Result<()> {
    let input = state.command_input.trim().to_string();
    state.command_input.clear();

    if input.is_empty() {
        // Empty enter repeats last search
        if !state.last_search.is_empty() {
            let query = state.last_search.clone();
            search_forward(state, &query);
        }
        return Ok(());
    }

    // :number → go to line
    if let Some(rest) = input.strip_prefix(':') {
        if let Ok(n) = rest.trim().parse::<usize>() {
            state.current_buffer_mut().go_to_line(n.saturating_sub(1));
            state.set_status(&format!("Line {n}"));
            return Ok(());
        }
    }

    // Plain number → go to line
    if let Ok(n) = input.parse::<usize>() {
        state.current_buffer_mut().go_to_line(n.saturating_sub(1));
        state.set_status(&format!("Line {n}"));
        return Ok(());
    }

    // /pattern → search
    if let Some(pattern) = input.strip_prefix('/') {
        if pattern.is_empty() {
            // Repeat last search
            if !state.last_search.is_empty() {
                let query = state.last_search.clone();
                search_forward(state, &query);
            }
        } else {
            state.last_search = pattern.to_string();
            let query = pattern.to_string();
            search_forward(state, &query);
        }
        return Ok(());
    }

    // goto <symbol> → search for function/symbol definition
    if let Some(symbol) = input
        .strip_prefix("goto ")
        .or_else(|| input.strip_prefix("g "))
    {
        let symbol = symbol.trim();
        if !symbol.is_empty() {
            goto_symbol(state, symbol);
            return Ok(());
        }
    }

    // Named commands
    match input.to_lowercase().as_str() {
        "exit" | "quit" | "q" => {
            state.quit = true;
        }
        "save" | "s" | "w" => {
            let result = state.current_buffer_mut().save();
            match result {
                Ok(()) => {
                    state.clear_external_conflict_for_current_buffer();
                    state.set_status("Saved");
                }
                Err(e) => state.set_status(&format!("Save failed: {e}")),
            }
        }
        "open" | "o" => {
            let picker = file_picker(&state.root_dir);
            state.picker = Some(ActivePicker::File(picker));
            state.mode = AppMode::Picker;
        }
        "diff" | "d" => {
            state.diff_mode = !state.diff_mode;
            if state.diff_mode {
                state.compute_diff_for_current();
                state.set_status("Diff view on");
            } else {
                state.set_status("Diff view off");
            }
        }
        "close" => {
            if state.split_buffers.is_some() {
                state.exit_split();
                state.set_status("Exited compare view");
            } else {
                close_or_confirm_tab(state);
            }
        }
        "help" | "?" => {
            state.help_visible = true;
            state.mode = AppMode::Help;
        }
        "sidebar" => {
            state.sidebar_visible = !state.sidebar_visible;
        }
        "wrap" => {
            state.wrap_lines = !state.wrap_lines;
            state.set_status(if state.wrap_lines {
                "Word wrap on"
            } else {
                "Word wrap off"
            });
        }
        "next" => {
            if !state.buffers.is_empty() {
                state.active_buffer = (state.active_buffer + 1) % state.buffers.len();
                state.close_dirty_confirm = false;
            }
        }
        "prev" => {
            if !state.buffers.is_empty() {
                state.active_buffer = if state.active_buffer == 0 {
                    state.buffers.len() - 1
                } else {
                    state.active_buffer - 1
                };
                state.close_dirty_confirm = false;
            }
        }
        "top" => {
            state.current_buffer_mut().go_to_line(0);
            state.set_status("Top of file");
        }
        "bottom" | "bot" => {
            let last = state.current_buffer().line_count().saturating_sub(1);
            state.current_buffer_mut().go_to_line(last);
            state.set_status("Bottom of file");
        }
        "n" => {
            // Next search result
            if !state.last_search.is_empty() {
                let query = state.last_search.clone();
                search_forward(state, &query);
            } else {
                state.set_status("No previous search");
            }
        }
        "changes" => {
            let picker = core_picker::Picker::new(state.changed_files.clone());
            state.picker = Some(ActivePicker::ChangedFiles(picker));
            state.mode = AppMode::Picker;
        }
        "touched" => {
            if state.touched_files.is_empty() {
                state.set_status("No externally touched files yet");
            } else {
                let picker = core_picker::Picker::new(state.touched_files.clone());
                state.picker = Some(ActivePicker::TouchedFiles(picker));
                state.mode = AppMode::Picker;
            }
        }
        "conflicts" => {
            let conflicts: Vec<_> = state
                .touched_files
                .iter()
                .filter(|entry| entry.conflict)
                .cloned()
                .collect();
            if conflicts.is_empty() {
                state.set_status("No external edit conflicts");
            } else {
                let picker = core_picker::Picker::new(conflicts);
                state.picker = Some(ActivePicker::TouchedFiles(picker));
                state.mode = AppMode::Picker;
            }
        }
        "reload" => match state.force_reload_current_buffer() {
            Ok(true) => state.set_status("Reloaded from disk"),
            Ok(false) => state.set_status("Already up to date"),
            Err(error) => state.set_status(&format!("Reload failed: {error}")),
        },
        "agent" => match state.launch_edit_agent_in_active_terminal() {
            Ok(_) => state.set_status("Launched edit-agent"),
            Err(error) => state.set_status(&format!("Agent launch failed: {error}")),
        },
        "edit" => {
            state.editing = true;
            state.set_status("Edit mode");
        }
        _ => {
            if let Some(query) = input.strip_prefix("grep ") {
                open_grep_results(state, query.trim())?;
            } else if let Some(rest) = input.strip_prefix("compare ") {
                open_compare(state, rest)?;
            } else if let Some(raw_path) = input.strip_prefix("open ") {
                let path = state.root_dir.join(raw_path.trim());
                state.open_file(&path)?;
            } else {
                state.set_status(&format!("Unknown command: {input}"));
            }
        }
    }

    Ok(())
}

fn search_forward(state: &mut AppState, query: &str) {
    let buf = &state.buffers[state.active_buffer];
    let content = buf.content();
    let cursor_byte = buf.cursor_byte_offset();

    // Search forward from just after cursor position
    let search_start = if cursor_byte + 1 < content.len() {
        cursor_byte + 1
    } else {
        cursor_byte
    };

    if let Some(pos) = content[search_start..].find(query) {
        let abs_pos = search_start + pos;
        jump_to_byte_offset(state, &content, abs_pos);
        state.set_status(&format!("Found '{query}'"));
    } else if let Some(pos) = content[..cursor_byte].find(query) {
        // Wrap around
        jump_to_byte_offset(state, &content, pos);
        state.set_status(&format!("Found '{query}' (wrapped)"));
    } else {
        state.set_status(&format!("Not found: '{query}'"));
    }
}

fn search_backward(state: &mut AppState, query: &str) {
    let buf = &state.buffers[state.active_buffer];
    let content = buf.content();
    let cursor_byte = buf.cursor_byte_offset();

    // Search backward from cursor
    if let Some(pos) = content[..cursor_byte].rfind(query) {
        jump_to_byte_offset(state, &content, pos);
        state.set_status(&format!("Found '{query}'"));
    } else if let Some(pos) = content[cursor_byte..].rfind(query) {
        // Wrap around
        let abs_pos = cursor_byte + pos;
        jump_to_byte_offset(state, &content, abs_pos);
        state.set_status(&format!("Found '{query}' (wrapped)"));
    } else {
        state.set_status(&format!("Not found: '{query}'"));
    }
}

fn jump_to_byte_offset(state: &mut AppState, content: &str, byte_pos: usize) {
    let mut line = 0;
    let mut col = 0;
    for (i, ch) in content.char_indices() {
        if i >= byte_pos {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    state.current_buffer_mut().cursor_line = line;
    state.current_buffer_mut().cursor_col = col;
}

fn goto_symbol(state: &mut AppState, symbol: &str) {
    let buf = &state.buffers[state.active_buffer];
    let content = buf.content();
    let lang = buf.language.as_str();

    // Build language-specific patterns for function/symbol definitions
    let patterns: Vec<String> = match lang {
        "rust" => vec![
            format!("fn {symbol}"),
            format!("struct {symbol}"),
            format!("enum {symbol}"),
            format!("trait {symbol}"),
            format!("impl {symbol}"),
            format!("mod {symbol}"),
            format!("type {symbol}"),
            format!("const {symbol}"),
            format!("static {symbol}"),
        ],
        "python" => vec![format!("def {symbol}"), format!("class {symbol}")],
        "javascript" | "typescript" => vec![
            format!("function {symbol}"),
            format!("class {symbol}"),
            format!("const {symbol}"),
            format!("let {symbol}"),
            format!("var {symbol}"),
        ],
        "go" => vec![format!("func {symbol}"), format!("type {symbol}")],
        _ => vec![symbol.to_string()],
    };

    // Search for the first matching pattern
    for pattern in &patterns {
        if let Some(pos) = content.find(pattern.as_str()) {
            jump_to_byte_offset(state, &content, pos);
            state.set_status(&format!("Found: {pattern}"));
            return;
        }
    }

    // Fallback: plain text search
    if let Some(pos) = content.find(symbol) {
        jump_to_byte_offset(state, &content, pos);
        state.set_status(&format!("Found '{symbol}'"));
    } else {
        state.set_status(&format!("Symbol not found: '{symbol}'"));
    }
}

fn handle_help_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            state.help_visible = false;
            state.mode = AppMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

fn handle_edit_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => state.quit = true,
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => match state.current_buffer_mut().save() {
            Ok(()) => {
                state.clear_external_conflict_for_current_buffer();
                state.set_status("Saved");
            }
            Err(e) => state.set_status(&format!("Save failed: {e}")),
        },
        (mods, KeyCode::Char('Z')) if mods.contains(KeyModifiers::CONTROL) => {
            if state.current_buffer_mut().redo() {
                state.set_status("Redo");
            }
        }
        (mods, KeyCode::Char('z')) if mods == KeyModifiers::CONTROL => {
            if state.current_buffer_mut().undo() {
                state.set_status("Undo");
            }
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            state.editing = false;
            state.set_status("View mode");
        }
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            state.pin_preview_buffer();
            state.current_buffer_mut().backspace();
        }
        (KeyModifiers::NONE, KeyCode::Delete) => {
            state.pin_preview_buffer();
            state.current_buffer_mut().delete_char();
        }
        (KeyModifiers::NONE, KeyCode::Enter) => {
            state.pin_preview_buffer();
            state.current_buffer_mut().new_line();
        }
        (KeyModifiers::NONE, KeyCode::Tab) => {
            state.pin_preview_buffer();
            state.current_buffer_mut().insert_char('\t');
        }
        (KeyModifiers::NONE, KeyCode::Left) => {
            state.current_buffer_mut().move_cursor(Direction::Left, 1)
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            state.current_buffer_mut().move_cursor(Direction::Right, 1)
        }
        (KeyModifiers::NONE, KeyCode::Up) => {
            state.current_buffer_mut().move_cursor(Direction::Up, 1)
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            state.current_buffer_mut().move_cursor(Direction::Down, 1)
        }
        (KeyModifiers::NONE, KeyCode::Char(ch)) | (KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
            state.pin_preview_buffer();
            state.current_buffer_mut().insert_char(ch);
        }
        _ => {}
    }
    Ok(())
}

fn handle_picker_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc => {
            state.picker = None;
            state.mode = AppMode::Normal;
        }
        KeyCode::Enter => {
            if let Some(ref picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => {
                        if let Some(picker_path) = p.selected_item() {
                            let full_path = state.root_dir.join(picker_path.as_path());
                            let _ = state.open_file(&full_path);
                        }
                    }
                    ActivePicker::ChangedFiles(p) => {
                        if let Some(changed) = p.selected_item() {
                            let full_path = state.root_dir.join(&changed.path);
                            let _ = state.open_file(&full_path);
                        }
                    }
                    ActivePicker::GrepResults(p) => {
                        if let Some(found) = p.selected_item().cloned() {
                            let full_path = state.root_dir.join(&found.path);
                            let _ = state.open_file(&full_path);
                            state
                                .current_buffer_mut()
                                .go_to_line(found.line.saturating_sub(1));
                            state.current_buffer_mut().cursor_col = found.column.saturating_sub(1);
                            state.set_status(&format!("{}:{}", found.path.display(), found.line));
                        }
                    }
                    ActivePicker::TouchedFiles(p) => {
                        if let Some(touched) = p.selected_item().cloned() {
                            let _ = state.open_file(&touched.path);
                            if touched.conflict {
                                state.set_status(
                                    "Opened touched file with external changes. Use `reload` to discard local edits.",
                                );
                            } else {
                                state.set_status(&format!("Opened touched file: {}", touched.display_path));
                            }
                        }
                    }
                }
            }
            state.picker = None;
            state.mode = AppMode::Normal;
        }
        KeyCode::Up => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.move_selection(-1),
                    ActivePicker::ChangedFiles(p) => p.move_selection(-1),
                    ActivePicker::GrepResults(p) => p.move_selection(-1),
                    ActivePicker::TouchedFiles(p) => p.move_selection(-1),
                }
            }
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.move_selection(1),
                    ActivePicker::ChangedFiles(p) => p.move_selection(1),
                    ActivePicker::GrepResults(p) => p.move_selection(1),
                    ActivePicker::TouchedFiles(p) => p.move_selection(1),
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.pop_char(),
                    ActivePicker::ChangedFiles(p) => p.pop_char(),
                    ActivePicker::GrepResults(p) => p.pop_char(),
                    ActivePicker::TouchedFiles(p) => p.pop_char(),
                }
            }
        }
        KeyCode::Char(ch) => {
            if key.modifiers == KeyModifiers::CONTROL {
                if let Some(ref picker) = state.picker {
                    if let ActivePicker::TouchedFiles(p) = picker {
                        if let Some(touched) = p.selected_item().cloned() {
                            match ch {
                                'd' | 'D' => {
                                    let _ = state.open_file(&touched.path);
                                    state.diff_mode = true;
                                    state.compute_diff_for_current();
                                    state.set_status(&format!(
                                        "Diffing touched file: {}",
                                        touched.display_path
                                    ));
                                    state.picker = None;
                                    state.mode = AppMode::Normal;
                                    return Ok(());
                                }
                                'r' | 'R' => {
                                    state.file_tree.reveal_path(&touched.path);
                                    state.focus_sidebar();
                                    state.set_status(&format!(
                                        "Revealed touched file: {}",
                                        touched.display_path
                                    ));
                                    state.picker = None;
                                    state.mode = AppMode::Normal;
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                    }
                }
                return Ok(());
            }
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.push_char(ch),
                    ActivePicker::ChangedFiles(p) => p.push_char(ch),
                    ActivePicker::GrepResults(p) => p.push_char(ch),
                    ActivePicker::TouchedFiles(p) => p.push_char(ch),
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn close_or_confirm_tab(state: &mut AppState) {
    if state.current_buffer().dirty && !state.close_dirty_confirm {
        state.close_dirty_confirm = true;
        state.set_status("Unsaved changes. Press Ctrl-W again to close");
        return;
    }
    state.close_dirty_confirm = false;
    state.close_active_tab();
}

fn open_grep_results(state: &mut AppState, query: &str) -> anyhow::Result<()> {
    if query.is_empty() {
        state.set_status("Usage: grep <pattern>");
        return Ok(());
    }

    let output = Command::new("rg")
        .arg("--vimgrep")
        .arg("--smart-case")
        .arg(query)
        .current_dir(&state.root_dir)
        .output()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut matches = Vec::new();
    for line in stdout.lines() {
        let mut parts = line.splitn(4, ':');
        let Some(path) = parts.next() else { continue };
        let Some(line_no) = parts.next() else {
            continue;
        };
        let Some(column_no) = parts.next() else {
            continue;
        };
        let Some(text) = parts.next() else { continue };
        let Ok(line_no) = line_no.parse::<usize>() else {
            continue;
        };
        let Ok(column_no) = column_no.parse::<usize>() else {
            continue;
        };
        matches.push(SearchMatch {
            path: path.into(),
            line: line_no,
            column: column_no,
            text: text.trim().to_string(),
        });
    }

    if matches.is_empty() {
        state.set_status(&format!("No matches for '{query}'"));
        return Ok(());
    }

    state.picker = Some(ActivePicker::GrepResults(core_picker::Picker::new(matches)));
    state.mode = AppMode::Picker;
    state.set_status(&format!("{} matches", stdout.lines().count()));
    Ok(())
}

fn open_compare(state: &mut AppState, rest: &str) -> anyhow::Result<()> {
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.len() != 2 {
        state.set_status("Usage: compare <file1> <file2>");
        return Ok(());
    }

    let left = state.open_file_with_index(&state.root_dir.join(parts[0]))?;
    let right = state.open_file_with_index(&state.root_dir.join(parts[1]))?;
    state.enter_split(left, right);
    state.set_status("Compare view");
    Ok(())
}

fn handle_sidebar_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<bool> {
    match key.code {
        KeyCode::Tab | KeyCode::Esc => {
            state.focus_editor();
        }
        KeyCode::Home => {
            state.file_tree.select_first();
            preview_selected_sidebar_file(state);
        }
        KeyCode::End => {
            state.file_tree.select_last();
            preview_selected_sidebar_file(state);
        }
        KeyCode::PageUp => {
            state
                .file_tree
                .move_selection(-(state.viewport_height.max(1) as i32));
            preview_selected_sidebar_file(state);
        }
        KeyCode::PageDown => {
            state
                .file_tree
                .move_selection(state.viewport_height.max(1) as i32);
            preview_selected_sidebar_file(state);
        }
        KeyCode::Up => {
            state.file_tree.move_selection(-1);
            preview_selected_sidebar_file(state);
        }
        KeyCode::Down => {
            state.file_tree.move_selection(1);
            preview_selected_sidebar_file(state);
        }
        KeyCode::Left => {
            if state.file_tree.collapse_selected_or_select_parent() {
                preview_selected_sidebar_file(state);
            }
        }
        KeyCode::Right => {
            if let Some(entry) = state.file_tree.selected_entry() {
                if entry.is_dir {
                    state.file_tree.expand_selected_or_select_child();
                    preview_selected_sidebar_file(state);
                } else {
                    preview_selected_sidebar_file(state);
                }
            }
        }
        KeyCode::Enter => {
            let selected = state.file_tree.selected;
            if let Some(entry) = state.file_tree.selected_entry() {
                if entry.is_dir {
                    state.file_tree.toggle_expand(selected);
                } else {
                    let path = entry.path.clone();
                    state.focus_editor();
                    if let Err(error) = state.open_file(&path) {
                        state.set_status(&format!("Open failed: {error}"));
                    }
                }
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn preview_selected_sidebar_file(state: &mut AppState) {
    let Some(entry) = state.file_tree.selected_entry() else {
        return;
    };
    if entry.is_dir {
        return;
    }
    let path = entry.path.clone();
    if let Err(error) = state.preview_file(&path) {
        state.set_status(&format!("Open failed: {error}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{execute_command_input, handle_key};
    use crate::state::AppState;
    use crate::workspace::FocusTarget;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn compare_command_enters_split_mode() {
        let dir = test_dir("compare");
        fs::write(dir.join("left.rs"), "fn left() {}\n").unwrap();
        fs::write(dir.join("right.rs"), "fn right() {}\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.command_input = "compare left.rs right.rs".to_string();
        execute_command_input(&mut state).unwrap();

        assert_eq!(state.split_buffers, Some((1, 2)));
        assert_eq!(state.active_buffer, 1);
    }

    #[test]
    fn edit_mode_routes_input_and_undo_redo() {
        let dir = test_dir("edit");
        fs::write(dir.join("file.txt"), "abc").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.command_input = "edit".to_string();
        execute_command_input(&mut state).unwrap();
        assert!(state.editing);

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
        )
        .unwrap();
        assert_eq!(state.current_buffer().content(), "dabc");

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(state.current_buffer().content(), "abc");

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('Z'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(state.current_buffer().content(), "dabc");
    }

    #[test]
    fn escape_exits_split_mode() {
        let dir = test_dir("split-escape");
        fs::write(dir.join("left.rs"), "fn left() {}\n").unwrap();
        fs::write(dir.join("right.rs"), "fn right() {}\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.command_input = "compare left.rs right.rs".to_string();
        execute_command_input(&mut state).unwrap();
        assert!(state.split_buffers.is_some());

        handle_key(&mut state, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        assert!(state.split_buffers.is_none());
    }

    #[test]
    fn f6_toggles_between_editor_and_terminal_workspace() {
        let dir = test_dir("workspace-toggle");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        assert!(matches!(state.focus_target, FocusTarget::Editor));

        handle_key(&mut state, KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).unwrap();
        assert!(matches!(state.focus_target, FocusTarget::TerminalPane(_)));

        handle_key(&mut state, KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).unwrap();
        assert!(matches!(state.focus_target, FocusTarget::Editor));
    }

    #[test]
    fn terminal_focus_does_not_append_to_command_input() {
        let dir = test_dir("terminal-placeholder");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.focus_terminal();

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        )
        .unwrap();

        assert!(state.command_input.is_empty());
        assert!(matches!(state.focus_target, FocusTarget::TerminalPane(_)));
    }

    #[test]
    fn terminal_split_hotkeys_manage_panes() {
        let dir = test_dir("terminal-splits");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.focus_terminal();

        handle_key(&mut state, KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).unwrap();
        assert_eq!(state.terminal_workspace.panes().len(), 2);

        handle_key(&mut state, KeyEvent::new(KeyCode::F(11), KeyModifiers::NONE)).unwrap();
        assert_eq!(state.terminal_workspace.panes().len(), 3);

        handle_key(&mut state, KeyEvent::new(KeyCode::F(12), KeyModifiers::NONE)).unwrap();
        assert_eq!(state.terminal_workspace.panes().len(), 2);
    }

    #[test]
    fn terminal_ctrl_tab_cycles_active_terminal_pane() {
        let dir = test_dir("terminal-cycle");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.join("file.txt")]).unwrap();
        state.focus_terminal();
        handle_key(&mut state, KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)).unwrap();
        let current = state.terminal_workspace.active_pane_id();

        handle_key(&mut state, KeyEvent::new(KeyCode::Tab, KeyModifiers::CONTROL)).unwrap();
        assert_ne!(state.terminal_workspace.active_pane_id(), current);
        assert!(matches!(state.focus_target, FocusTarget::TerminalPane(_)));
    }

    #[test]
    fn sidebar_navigation_previews_selected_file() {
        let dir = test_dir("sidebar-preview");
        fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        fs::write(dir.join("b.txt"), "beta\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();
        state.file_tree.selected = state
            .file_tree
            .entries
            .iter()
            .position(|entry| entry.name == "a.txt")
            .unwrap();

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        )
        .unwrap();

        assert!(state.sidebar_focused);
        assert_eq!(state.buffers.len(), 1);
        assert_eq!(state.preview_buffer, Some(0));
        assert_eq!(state.current_buffer().file_name(), "b.txt");
        assert_eq!(state.current_buffer().content(), "beta\n");
    }

    #[test]
    fn sidebar_enter_opens_file_and_returns_focus_to_editor() {
        let dir = test_dir("sidebar-enter");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();
        state.file_tree.selected = state
            .file_tree
            .entries
            .iter()
            .position(|entry| entry.name == "file.txt")
            .unwrap();

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        )
        .unwrap();

        assert!(!state.sidebar_focused);
        assert!(matches!(state.focus_target, FocusTarget::Editor));
        assert_eq!(state.preview_buffer, None);
        assert_eq!(state.current_buffer().file_name(), "file.txt");
    }

    #[test]
    fn sidebar_focus_still_allows_global_quit_shortcut() {
        let dir = test_dir("sidebar-global-quit");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();

        handle_key(
            &mut state,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        )
        .unwrap();

        assert!(state.quit);
    }

    #[test]
    fn sidebar_focus_still_allows_workspace_toggle_shortcut() {
        let dir = test_dir("sidebar-global-toggle");
        fs::write(dir.join("file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();

        handle_key(&mut state, KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)).unwrap();

        assert!(matches!(state.focus_target, FocusTarget::TerminalPane(_)));
    }

    #[test]
    fn sidebar_left_on_file_selects_parent_directory() {
        let dir = test_dir("sidebar-left-parent");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();
        state
            .file_tree
            .reveal_path(&dir.join("nested/file.txt"));

        handle_key(&mut state, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)).unwrap();

        let selected = state.file_tree.selected_entry().expect("selected entry");
        assert_eq!(selected.name, "nested");
        assert!(selected.is_dir);
    }

    #[test]
    fn sidebar_right_on_directory_selects_first_child() {
        let dir = test_dir("sidebar-right-child");
        fs::create_dir_all(dir.join("nested")).unwrap();
        fs::write(dir.join("nested/file.txt"), "hello\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.focus_sidebar();
        state.file_tree.reveal_path(&dir.join("nested"));

        handle_key(&mut state, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)).unwrap();
        handle_key(&mut state, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)).unwrap();

        assert_eq!(state.current_buffer().file_name(), "file.txt");
        let selected = state.file_tree.selected_entry().expect("selected entry");
        assert_eq!(selected.name, "file.txt");
        assert!(!selected.is_dir);
    }

    #[test]
    fn sidebar_preview_exits_compare_view() {
        let dir = test_dir("sidebar-preview-compare");
        fs::write(dir.join("left.txt"), "left\n").unwrap();
        fs::write(dir.join("right.txt"), "right\n").unwrap();
        fs::write(dir.join("third.txt"), "third\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        state.command_input = "compare left.txt right.txt".to_string();
        execute_command_input(&mut state).unwrap();
        state.focus_sidebar();
        state.file_tree.selected = state
            .file_tree
            .entries
            .iter()
            .position(|entry| entry.name == "third.txt")
            .unwrap();

        handle_key(&mut state, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)).unwrap();

        assert!(state.split_buffers.is_none());
        assert_eq!(state.current_buffer().file_name(), "third.txt");
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("edit-tests-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
