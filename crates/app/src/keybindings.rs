use crate::state::{ActivePicker, AppMode, AppState};
use core_buffer::Direction;
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

    // Normal mode: check sidebar focus
    if state.sidebar_focused && state.sidebar_visible {
        return handle_sidebar_key(state, key);
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
                Ok(()) => state.set_status("Saved"),
                Err(e) => state.set_status(&format!("Save failed: {e}")),
            }
        }

        // Toggle sidebar
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            state.sidebar_visible = !state.sidebar_visible;
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
                state.sidebar_focused = true;
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
                Ok(()) => state.set_status("Saved"),
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
            Ok(()) => state.set_status("Saved"),
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
        (KeyModifiers::NONE, KeyCode::Backspace) => state.current_buffer_mut().backspace(),
        (KeyModifiers::NONE, KeyCode::Delete) => state.current_buffer_mut().delete_char(),
        (KeyModifiers::NONE, KeyCode::Enter) => state.current_buffer_mut().new_line(),
        (KeyModifiers::NONE, KeyCode::Tab) => state.current_buffer_mut().insert_char('\t'),
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
                }
            }
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.move_selection(1),
                    ActivePicker::ChangedFiles(p) => p.move_selection(1),
                    ActivePicker::GrepResults(p) => p.move_selection(1),
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.pop_char(),
                    ActivePicker::ChangedFiles(p) => p.pop_char(),
                    ActivePicker::GrepResults(p) => p.pop_char(),
                }
            }
        }
        KeyCode::Char(ch) => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.push_char(ch),
                    ActivePicker::ChangedFiles(p) => p.push_char(ch),
                    ActivePicker::GrepResults(p) => p.push_char(ch),
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

fn handle_sidebar_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Tab | KeyCode::Esc => {
            state.sidebar_focused = false;
        }
        KeyCode::Up => {
            state.file_tree.move_selection(-1);
        }
        KeyCode::Down => {
            state.file_tree.move_selection(1);
        }
        KeyCode::Enter => {
            let selected = state.file_tree.selected;
            if let Some(entry) = state.file_tree.selected_entry() {
                if entry.is_dir {
                    state.file_tree.toggle_expand(selected);
                } else {
                    let path = entry.path.clone();
                    state.sidebar_focused = false;
                    let _ = state.open_file(&path);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{execute_command_input, handle_key};
    use crate::state::AppState;
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
