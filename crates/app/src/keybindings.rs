use crate::state::{ActivePicker, AppMode, AppState};
use core_buffer::Direction;
use core_picker::file_picker;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub fn handle_key(state: &mut AppState, key: KeyEvent) -> anyhow::Result<()> {
    match state.mode {
        AppMode::Help => return handle_help_key(state, key),
        AppMode::Picker => return handle_picker_key(state, key),
        AppMode::Normal => {}
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
                let buf = &state.buffers[state.active_buffer];
                let path_clone = buf.path.clone();
                let current_line = buf.cursor_line + 1;
                if let Some(ref path) = path_clone {
                    let target_info = state.diffs.get(path).and_then(|diff| {
                        diff.hunks.iter().find(|h| h.new_start > current_line)
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

        // Tab → sidebar focus
        (KeyModifiers::NONE, KeyCode::Tab) => {
            if state.sidebar_visible {
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
            }
        }

        // Backspace → delete from command input
        (KeyModifiers::NONE, KeyCode::Backspace) => {
            state.command_input.pop();
        }

        // All printable characters → command input
        (KeyModifiers::NONE, KeyCode::Char(ch))
        | (KeyModifiers::SHIFT, KeyCode::Char(ch)) => {
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
    if let Some(symbol) = input.strip_prefix("goto ").or_else(|| input.strip_prefix("g ")) {
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
            state.close_active_tab();
        }
        "help" | "?" => {
            state.help_visible = true;
            state.mode = AppMode::Help;
        }
        "sidebar" => {
            state.sidebar_visible = !state.sidebar_visible;
        }
        "next" => {
            if !state.buffers.is_empty() {
                state.active_buffer = (state.active_buffer + 1) % state.buffers.len();
            }
        }
        "prev" => {
            if !state.buffers.is_empty() {
                state.active_buffer = if state.active_buffer == 0 {
                    state.buffers.len() - 1
                } else {
                    state.active_buffer - 1
                };
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
        _ => {
            // Try as a search (if it contains non-command text)
            state.set_status(&format!("Unknown command: {input}"));
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
        "python" => vec![
            format!("def {symbol}"),
            format!("class {symbol}"),
        ],
        "javascript" | "typescript" => vec![
            format!("function {symbol}"),
            format!("class {symbol}"),
            format!("const {symbol}"),
            format!("let {symbol}"),
            format!("var {symbol}"),
        ],
        "go" => vec![
            format!("func {symbol}"),
            format!("type {symbol}"),
        ],
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
