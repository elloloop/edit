use crate::keybindings;
use crate::state::{ActivePicker, AppMode, AppState};
use core_buffer::Direction;
use core_fs::FileEvent;
use crossterm::event::{self, Event, KeyEventKind, MouseButton, MouseEvent, MouseEventKind};
use ratatui::backend::Backend;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::collections::HashSet;
use std::io::Stdout;
use std::time::Duration;
use ui_tui::layout;

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
            &state.command_input,
            status_msg,
            &state.breadcrumb(),
            &state.last_search,
            state.matching_bracket(),
            state.wrap_lines,
            state.editing,
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

    for path in changed_paths {
        state.reload_if_open(&path);
    }

    if tree_dirty {
        state.refresh_workspace();
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

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if state.sidebar_focused {
                state.file_tree.move_selection(-(scroll_lines as i32));
            } else {
                state
                    .current_buffer_mut()
                    .move_cursor(Direction::Up, scroll_lines);
            }
        }
        MouseEventKind::ScrollDown => {
            if state.sidebar_focused {
                state.file_tree.move_selection(scroll_lines as i32);
            } else {
                state
                    .current_buffer_mut()
                    .move_cursor(Direction::Down, scroll_lines);
            }
        }
        MouseEventKind::Down(MouseButton::Left)
            if state.sidebar_visible
                && mouse.column < 30
                && mouse.row >= body_top
                && mouse.row < body_top + body_height =>
        {
            let height = body_height as usize;
            let scroll = if state.file_tree.selected >= height {
                state.file_tree.selected - height + 1
            } else {
                0
            };
            let clicked = scroll + (mouse.row - body_top) as usize;
            if clicked < state.file_tree.entries.len() {
                state.sidebar_focused = true;
                state.file_tree.selected = clicked;
                if let Some(entry) = state.file_tree.selected_entry() {
                    if entry.is_dir {
                        state.file_tree.toggle_expand(clicked);
                    } else {
                        let path = entry.path.clone();
                        state.sidebar_focused = false;
                        let _ = state.open_file(&path);
                    }
                }
            }
        }
        _ => {}
    }
}
