use crate::state::{ActivePicker, AppMode, AppState};
use core_buffer::Direction;
use core_picker::{command_picker, file_picker};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    // Handle mode-specific keys first
    match state.mode {
        AppMode::Search => return handle_search_key(state, key),
        AppMode::GoToLine => return handle_goto_key(state, key),
        AppMode::Help => return handle_help_key(state, key),
        AppMode::Picker => return handle_picker_key(state, key),
        AppMode::Command => return handle_command_key(state, key),
        AppMode::Normal => {}
    }

    // Normal mode: check sidebar focus
    if state.sidebar_focused && state.sidebar_visible {
        return handle_sidebar_key(state, key);
    }

    match (key.modifiers, key.code) {
        // Quit
        (KeyModifiers::CONTROL, KeyCode::Char('q')) => {
            state.quit = true;
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

        // Go to line
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            state.goto_input.clear();
            state.mode = AppMode::GoToLine;
        }

        // Toggle diff
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            state.diff_mode = !state.diff_mode;
            if state.diff_mode {
                state.compute_diff_for_current();
            }
        }

        // Close tab
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            state.close_active_tab();
        }

        // Next tab
        (KeyModifiers::CONTROL, KeyCode::Tab)
        | (KeyModifiers::NONE, KeyCode::F(2)) => {
            if !state.buffers.is_empty() {
                state.active_buffer = (state.active_buffer + 1) % state.buffers.len();
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
            }
        }

        // Next diff hunk
        (KeyModifiers::NONE, KeyCode::F(8)) => {
            if state.diff_mode {
                state.set_status("Next diff hunk");
                // Jump to next hunk line if we have a diff
                let buf = &state.buffers[state.active_buffer];
                if let Some(ref path) = buf.path.clone() {
                    if let Some(diff) = state.diffs.get(path) {
                        let current_line = buf.cursor_line + 1; // 1-indexed
                        for hunk in &diff.hunks {
                            if hunk.new_start > current_line {
                                let target = hunk.new_start.saturating_sub(1);
                                state.current_buffer_mut().go_to_line(target);
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Search
        (KeyModifiers::NONE, KeyCode::Char('/')) => {
            state.search_query.clear();
            state.mode = AppMode::Search;
        }

        // Command palette
        (KeyModifiers::NONE, KeyCode::Char(':'))
        | (KeyModifiers::SHIFT, KeyCode::Char(':')) => {
            state.command_palette = Some(command_picker());
            state.mode = AppMode::Command;
        }

        // Help
        (KeyModifiers::NONE, KeyCode::Char('?'))
        | (KeyModifiers::SHIFT, KeyCode::Char('?')) => {
            state.help_visible = !state.help_visible;
            if state.help_visible {
                state.mode = AppMode::Help;
            }
        }

        // Tab to focus sidebar
        (KeyModifiers::NONE, KeyCode::Tab) => {
            if state.sidebar_visible {
                state.sidebar_focused = true;
            }
        }

        // Movement
        (KeyModifiers::NONE, KeyCode::Up) => {
            state.current_buffer_mut().move_cursor(Direction::Up, 1);
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            state.current_buffer_mut().move_cursor(Direction::Down, 1);
        }
        (KeyModifiers::NONE, KeyCode::Left) => {
            state.current_buffer_mut().move_cursor(Direction::Left, 1);
        }
        (KeyModifiers::NONE, KeyCode::Right) => {
            state.current_buffer_mut().move_cursor(Direction::Right, 1);
        }
        (KeyModifiers::NONE, KeyCode::Home) => {
            state.current_buffer_mut().move_cursor(Direction::Home, 1);
        }
        (KeyModifiers::NONE, KeyCode::End) => {
            state.current_buffer_mut().move_cursor(Direction::End, 1);
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            state.current_buffer_mut().move_cursor(Direction::PageUp, 1);
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            state
                .current_buffer_mut()
                .move_cursor(Direction::PageDown, 1);
        }

        // Editing
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            state.current_buffer_mut().backspace();
            state.reparse_current_buffer();
        }
        (KeyModifiers::NONE, KeyCode::Delete) => {
            state.current_buffer_mut().delete_char();
            state.reparse_current_buffer();
        }
        (KeyModifiers::NONE, KeyCode::Enter) => {
            state.current_buffer_mut().new_line();
            state.reparse_current_buffer();
        }
        (KeyModifiers::NONE, KeyCode::Char(ch))
        | (KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
            state.current_buffer_mut().insert_char(ch);
            state.reparse_current_buffer();
        }

        _ => {}
    }

    Ok(())
}

fn handle_search_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc => {
            state.mode = AppMode::Normal;
            state.search_query.clear();
        }
        KeyCode::Enter => {
            // Perform search in current buffer
            let query = state.search_query.clone();
            if !query.is_empty() {
                let buf = &state.buffers[state.active_buffer];
                let content = buf.content();
                let cursor_byte = buf.cursor_byte_offset();
                // Search forward from cursor
                if let Some(pos) = content[cursor_byte..].find(&query) {
                    let abs_pos = cursor_byte + pos;
                    // Convert byte offset to line/col
                    let mut line = 0;
                    let mut col = 0;
                    for (i, ch) in content.char_indices() {
                        if i >= abs_pos {
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
                    state.set_status(&format!("Found '{query}'"));
                } else {
                    // Wrap around: search from beginning
                    if let Some(pos) = content.find(&query) {
                        let mut line = 0;
                        let mut col = 0;
                        for (i, ch) in content.char_indices() {
                            if i >= pos {
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
                        state.set_status(&format!("Found '{query}' (wrapped)"));
                    } else {
                        state.set_status(&format!("Not found: '{query}'"));
                    }
                }
            }
            state.mode = AppMode::Normal;
        }
        KeyCode::Backspace => {
            state.search_query.pop();
        }
        KeyCode::Char(ch) => {
            state.search_query.push(ch);
        }
        _ => {}
    }
    Ok(())
}

fn handle_goto_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc => {
            state.mode = AppMode::Normal;
            state.goto_input.clear();
        }
        KeyCode::Enter => {
            if let Ok(line_num) = state.goto_input.parse::<usize>() {
                let target = line_num.saturating_sub(1);
                state.current_buffer_mut().go_to_line(target);
                state.set_status(&format!("Jumped to line {line_num}"));
            }
            state.goto_input.clear();
            state.mode = AppMode::Normal;
        }
        KeyCode::Backspace => {
            state.goto_input.pop();
        }
        KeyCode::Char(ch) if ch.is_ascii_digit() => {
            state.goto_input.push(ch);
        }
        _ => {}
    }
    Ok(())
}

fn handle_help_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
            state.help_visible = false;
            state.mode = AppMode::Normal;
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
                }
            }
        }
        KeyCode::Down => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.move_selection(1),
                    ActivePicker::ChangedFiles(p) => p.move_selection(1),
                }
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.pop_char(),
                    ActivePicker::ChangedFiles(p) => p.pop_char(),
                }
            }
        }
        KeyCode::Char(ch) => {
            if let Some(ref mut picker) = state.picker {
                match picker {
                    ActivePicker::File(p) => p.push_char(ch),
                    ActivePicker::ChangedFiles(p) => p.push_char(ch),
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn handle_command_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match key.code {
        KeyCode::Esc => {
            state.command_palette = None;
            state.mode = AppMode::Normal;
        }
        KeyCode::Enter => {
            if let Some(ref palette) = state.command_palette {
                if let Some(cmd) = palette.selected_item() {
                    let name = cmd.name.clone();
                    state.command_palette = None;
                    state.mode = AppMode::Normal;
                    execute_command(state, &name)?;
                    return Ok(());
                }
            }
            state.command_palette = None;
            state.mode = AppMode::Normal;
        }
        KeyCode::Up => {
            if let Some(ref mut palette) = state.command_palette {
                palette.move_selection(-1);
            }
        }
        KeyCode::Down => {
            if let Some(ref mut palette) = state.command_palette {
                palette.move_selection(1);
            }
        }
        KeyCode::Backspace => {
            if let Some(ref mut palette) = state.command_palette {
                palette.pop_char();
            }
        }
        KeyCode::Char(ch) => {
            if let Some(ref mut palette) = state.command_palette {
                palette.push_char(ch);
            }
        }
        _ => {}
    }
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

fn execute_command(state: &mut AppState, name: &str) -> anyhow::Result<()> {
    match name {
        "Save" => {
            let result = state.current_buffer_mut().save();
            match result {
                Ok(()) => state.set_status("Saved"),
                Err(e) => state.set_status(&format!("Save failed: {e}")),
            }
        }
        "Close Tab" => {
            state.close_active_tab();
        }
        "Open File" => {
            let picker = file_picker(&state.root_dir);
            state.picker = Some(ActivePicker::File(picker));
            state.mode = AppMode::Picker;
        }
        "Toggle Sidebar" => {
            state.sidebar_visible = !state.sidebar_visible;
        }
        "Toggle Diff" => {
            state.diff_mode = !state.diff_mode;
            if state.diff_mode {
                state.compute_diff_for_current();
            }
        }
        "Go to Line" => {
            state.goto_input.clear();
            state.mode = AppMode::GoToLine;
        }
        "Help" => {
            state.help_visible = !state.help_visible;
            if state.help_visible {
                state.mode = AppMode::Help;
            }
        }
        "Quit" => {
            state.quit = true;
        }
        "Next Tab" => {
            if !state.buffers.is_empty() {
                state.active_buffer = (state.active_buffer + 1) % state.buffers.len();
            }
        }
        "Previous Tab" => {
            if !state.buffers.is_empty() {
                state.active_buffer = if state.active_buffer == 0 {
                    state.buffers.len() - 1
                } else {
                    state.active_buffer - 1
                };
            }
        }
        "Search" => {
            state.search_query.clear();
            state.mode = AppMode::Search;
        }
        "Next Diff Hunk" => {
            state.set_status("Next diff hunk");
        }
        _ => {
            state.set_status(&format!("Unknown command: {name}"));
        }
    }
    Ok(())
}
