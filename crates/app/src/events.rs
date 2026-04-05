use crate::keybindings;
use crate::state::{ActivePicker, AppMode, AppState};
use core_buffer::Direction;
use core_fs::FileEvent;
use crossterm::event::{self, Event, KeyEventKind, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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
        state.current_buffer_mut().ensure_cursor_visible(height);

        // Render
        terminal.draw(|f| {
            let file_picker = match &state.picker {
                Some(ActivePicker::File(p)) => Some(p),
                _ => None,
            };
            let status_msg = state
                .status_message
                .as_ref()
                .map(|(msg, _)| msg.as_str());

            layout::render_app(
                f,
                &state.buffers,
                state.active_buffer,
                &state.file_tree,
                state.sidebar_visible,
                &state.theme,
                &state.highlighters,
                state.diff_mode,
                &state.diffs,
                state.help_visible,
                file_picker,
                &state.command_input,
                status_msg,
            );
        })?;

        // Handle input events
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Press {
                        keybindings::handle_key(state, key)?;
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse(state, mouse.kind);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// Drain file watcher channel and reload any open buffers that changed on disk.
fn process_file_events(state: &mut AppState) {
    let mut changed_paths = HashSet::new();

    if let Some(ref rx) = state.file_events {
        while let Ok(event) = rx.try_recv() {
            match event {
                FileEvent::Modified(path) | FileEvent::Created(path) => {
                    changed_paths.insert(path);
                }
                FileEvent::Deleted(_) => {}
            }
        }
    }

    for path in changed_paths {
        state.reload_if_open(&path);
    }
}

fn handle_mouse(state: &mut AppState, kind: MouseEventKind) {
    if state.mode != AppMode::Normal {
        return;
    }

    let scroll_lines = 3;

    match kind {
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
        _ => {}
    }
}
