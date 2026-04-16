use crate::keybindings;
use crate::state::{ActivePicker, AppMode, AppState, ExternalChangeOutcome};
use crate::workspace::FocusTarget;
use core_buffer::Direction;
use core_fs::FileEvent;
use crossterm::event::{self, Event, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::Stdout;
use std::time::Duration;
use ui_tui::{layout, terminal_view};

pub fn run(
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> anyhow::Result<()> {
    loop {
        if state.quit {
            break;
        }

        // Clear expired status messages (after 3 seconds)
        if let Some((_, when)) = &state.status_message {
            if when.elapsed() > Duration::from_secs(3) {
                state.status_message = None;
            }
        }

        // Process file system events — auto-reload files changed by agents
        process_file_events(state);
        process_agent_events(state);
        state.terminal_runtime.poll_all();
        resize_active_terminal(state, terminal.size()?);

        // Ensure cursor is visible in viewport
        let height = terminal.size()?.height.saturating_sub(3) as usize;
        state.viewport_height = height;
        state.current_buffer_mut().ensure_cursor_visible(height);

        render_once(state, terminal)?;

        // Handle input events
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        keybindings::handle_key(state, key)?;
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse(state, terminal.size()?, mouse);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

pub fn render_once<B: Backend>(
    state: &mut AppState,
    terminal: &mut Terminal<B>,
) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let terminal_area = {
            let main_layout = ratatui::layout::Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Length(1),
                    ratatui::layout::Constraint::Min(1),
                    ratatui::layout::Constraint::Length(2),
                ])
                .split(f.area());
            let body_area = main_layout[2];
            let [terminal_area, _] =
                layout::split_root_workspace(body_area, state.root_terminal_ratio_percent);
            layout::workspace_panel_inner(terminal_area)
        };
        let terminal_panes = state.terminal_pane_renders(terminal_area);
        let workspace_summary = state.workspace_summary();
        let file_picker = match &state.picker {
            Some(ActivePicker::File(p)) => Some(p),
            _ => None,
        };
        let changed_picker = match &state.picker {
            Some(ActivePicker::ChangedFiles(p)) => Some(p),
            _ => None,
        };
        let grep_picker = match &state.picker {
            Some(ActivePicker::GrepResults(p)) => Some(p),
            _ => None,
        };
        let touched_picker = match &state.picker {
            Some(ActivePicker::TouchedFiles(p)) => Some(p),
            _ => None,
        };
        let status_msg = state.status_message.as_ref().map(|(msg, _)| msg.as_str());

        layout::render_app(
            f,
            &state.buffers,
            state.active_buffer,
            state.split_buffers,
            &state.file_tree,
            state.sidebar_visible,
            &state.theme,
            &state.highlighters,
            state.diff_mode,
            &state.diffs,
            state.help_visible,
            file_picker,
            changed_picker,
            grep_picker,
            touched_picker,
            &state.command_input,
            status_msg,
            &state.breadcrumb(),
            &state.last_search,
            state.matching_bracket(),
            state.wrap_lines,
            state.editing,
            &state.touched_paths,
            &state.external_conflicts,
            state.touched_files.len(),
            state.current_buffer_has_external_conflict(),
            workspace_summary.as_deref(),
            matches!(state.focus_target, FocusTarget::TerminalPane(_)),
            state.sidebar_focused,
            state.root_terminal_ratio_percent,
            |f, _inner| {
                terminal_view::render_terminal_workspace(f, &terminal_panes, &state.theme);
            },
        );
    })?;
    Ok(())
}

/// Drain file watcher channel and reload any open buffers that changed on disk.
fn process_file_events(state: &mut AppState) {
    let mut changed_paths = HashSet::new();
    let mut tree_dirty = false;

    if let Some(ref rx) = state.file_events {
        while let Ok(event) = rx.try_recv() {
            match event {
                FileEvent::Modified(path) | FileEvent::Created(path) => {
                    tree_dirty = true;
                    changed_paths.insert(path);
                }
                FileEvent::Deleted(_) => {
                    tree_dirty = true;
                }
            }
        }
    }

    let mut reloaded = Vec::new();
    let mut conflicts = Vec::new();

    for path in changed_paths {
        match state.handle_external_change(&path) {
            ExternalChangeOutcome::Reloaded(file_name) => reloaded.push(file_name),
            ExternalChangeOutcome::Conflict(file_name) => conflicts.push(file_name),
            ExternalChangeOutcome::Touched(_) => {}
        }
    }

    if tree_dirty {
        state.refresh_workspace();
    }

    if !conflicts.is_empty() {
        state.set_status(&format!(
            "External edits waiting for review: {}. Use `touched`, `conflicts`, or `reload`.",
            conflicts.join(", ")
        ));
    } else if let Some(file_name) = reloaded.last() {
        state.set_status(&format!("Reloaded: {file_name}"));
    }
}

fn process_agent_events(state: &mut AppState) {
    let mut events = Vec::new();
    if let Some(ref rx) = state.agent_events {
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
    }

    for event in events {
        state.process_agent_bridge_event(event);
    }
}

fn handle_mouse(state: &mut AppState, terminal_size: ratatui::layout::Size, mouse: MouseEvent) {
    if state.mode != AppMode::Normal {
        return;
    }

    let scroll_lines = 3;
    let body_top = 2u16;
    let command_height = 2u16;
    let body_height = terminal_size
        .height
        .saturating_sub(body_top + command_height);
    let body_area = ratatui::layout::Rect::new(0, body_top, terminal_size.width, body_height);
    let [terminal_area, editor_area] =
        layout::split_root_workspace(body_area, state.root_terminal_ratio_percent);
    let terminal_inner = layout::workspace_panel_inner(terminal_area);
    let sidebar_area = sidebar_hit_area(state, editor_area);

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if point_in_rect(mouse.column, mouse.row, terminal_area) {
                state.scroll_active_terminal(-(scroll_lines as isize));
            } else if sidebar_area
                .map(|area| point_in_rect(mouse.column, mouse.row, area))
                .unwrap_or(false)
            {
                state.file_tree.move_selection(-(scroll_lines as i32));
                preview_selected_sidebar_file(state);
            } else if state.terminal_workspace_focused() {
                state.scroll_active_terminal(-(scroll_lines as isize));
            } else if state.sidebar_focused {
                state.file_tree.move_selection(-(scroll_lines as i32));
                preview_selected_sidebar_file(state);
            } else {
                state
                    .current_buffer_mut()
                    .move_cursor(Direction::Up, scroll_lines);
            }
        }
        MouseEventKind::ScrollDown => {
            if point_in_rect(mouse.column, mouse.row, terminal_area) {
                state.scroll_active_terminal(scroll_lines as isize);
            } else if sidebar_area
                .map(|area| point_in_rect(mouse.column, mouse.row, area))
                .unwrap_or(false)
            {
                state.file_tree.move_selection(scroll_lines as i32);
                preview_selected_sidebar_file(state);
            } else if state.terminal_workspace_focused() {
                state.scroll_active_terminal(scroll_lines as isize);
            } else if state.sidebar_focused {
                state.file_tree.move_selection(scroll_lines as i32);
                preview_selected_sidebar_file(state);
            } else {
                state
                    .current_buffer_mut()
                    .move_cursor(Direction::Down, scroll_lines);
            }
        }
        MouseEventKind::Down(MouseButton::Left)
            if point_in_rect(mouse.column, mouse.row, terminal_area) =>
        {
            if let Some(layout) = state
                .terminal_workspace
                .layout_rects(terminal_inner)
                .into_iter()
                .find(|layout| point_in_rect(mouse.column, mouse.row, layout.area))
            {
                state.terminal_workspace.select_pane(layout.pane_id);
            }
            state.focus_terminal();
        }
        MouseEventKind::Down(MouseButton::Left)
            if point_in_rect(mouse.column, mouse.row, editor_area) =>
        {
            state.focus_editor();

            if state.sidebar_visible {
                if let Some(sidebar_area) = sidebar_area {
                    let height = sidebar_area.height as usize;
                    let scroll = if state.file_tree.selected >= height {
                        state.file_tree.selected - height + 1
                    } else {
                        0
                    };
                    let clicked = scroll + (mouse.row - sidebar_area.y) as usize;
                    if clicked < state.file_tree.entries.len() {
                        state.focus_sidebar();
                        state.file_tree.selected = clicked;
                        if let Some(entry) = state.file_tree.selected_entry() {
                            if entry.is_dir {
                                state.file_tree.toggle_expand(clicked);
                            } else {
                                let path = entry.path.clone();
                                if let Err(error) = state.preview_file(&path) {
                                    state.set_status(&format!("Open failed: {error}"));
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }
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

fn sidebar_hit_area(state: &AppState, editor_area: ratatui::layout::Rect) -> Option<ratatui::layout::Rect> {
    if !state.sidebar_visible {
        return None;
    }

    Some(ratatui::layout::Rect::new(
        editor_area.x.saturating_add(1),
        editor_area.y.saturating_add(1),
        30,
        editor_area.height.saturating_sub(2),
    ))
}

fn point_in_rect(column: u16, row: u16, rect: ratatui::layout::Rect) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

fn resize_active_terminal(state: &mut AppState, size: ratatui::layout::Size) {
    let body_height = size.height.saturating_sub(4);
    let body_area = ratatui::layout::Rect::new(0, 2, size.width, body_height);
    let [terminal_area, _] = layout::split_root_workspace(body_area, state.root_terminal_ratio_percent);
    let terminal_inner = layout::workspace_panel_inner(terminal_area);
    state.resize_terminals_in_workspace(terminal_inner);
}

#[cfg(test)]
mod tests {
    use super::handle_mouse;
    use crate::state::AppState;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use ratatui::layout::Size;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn clicking_sidebar_file_previews_it_and_keeps_sidebar_focus() {
        let dir = test_dir("mouse-sidebar");
        fs::write(dir.join("alpha.txt"), "alpha\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        let file_index = state
            .file_tree
            .entries
            .iter()
            .position(|entry| entry.name == "alpha.txt")
            .unwrap();

        let size = Size::new(120, 40);
        let body_area = ratatui::layout::Rect::new(0, 2, size.width, size.height.saturating_sub(4));
        let [_terminal_area, editor_area] =
            super::layout::split_root_workspace(body_area, state.root_terminal_ratio_percent);
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: editor_area.x + 2,
            row: editor_area.y + 1 + file_index as u16,
            modifiers: KeyModifiers::NONE,
        };

        handle_mouse(&mut state, size, click);

        assert!(state.sidebar_focused);
        assert_eq!(state.file_tree.selected, file_index);
        assert_eq!(state.current_buffer().file_name(), "alpha.txt");
    }

    #[test]
    fn scrolling_sidebar_previews_selected_file_without_sidebar_focus() {
        let dir = test_dir("mouse-sidebar-scroll");
        fs::write(dir.join("alpha.txt"), "alpha\n").unwrap();
        fs::write(dir.join("beta.txt"), "beta\n").unwrap();

        let mut state = AppState::new(vec![dir.clone()]).unwrap();
        let size = Size::new(120, 40);
        let body_area =
            ratatui::layout::Rect::new(0, 2, size.width, size.height.saturating_sub(4));
        let [_terminal_area, editor_area] =
            super::layout::split_root_workspace(body_area, state.root_terminal_ratio_percent);
        let scroll = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: editor_area.x + 2,
            row: editor_area.y + 2,
            modifiers: KeyModifiers::NONE,
        };

        handle_mouse(&mut state, size, scroll);

        assert_eq!(state.current_buffer().file_name(), "beta.txt");
        assert_eq!(state.preview_buffer, Some(0));
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("edit-events-tests-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
