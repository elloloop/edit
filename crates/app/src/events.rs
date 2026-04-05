use crate::keybindings;
use crate::state::{ActivePicker, AppState};
use crossterm::event::{self, Event, KeyEventKind};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
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

        // Ensure cursor is visible in viewport
        let height = terminal.size()?.height.saturating_sub(3) as usize; // tabs + command bar (2 lines)
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

        // Handle events
        if event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Only handle key press events (not release/repeat on some terminals)
                if key.kind == KeyEventKind::Press {
                    keybindings::handle_key(state, key)?;
                }
            }
        }
    }

    Ok(())
}
