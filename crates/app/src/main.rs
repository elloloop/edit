mod agent_bridge;
mod events;
mod keybindings;
mod state;
mod terminal_input;
mod workspace;

use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::Terminal;
use state::AppState;
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let benchmark = args.iter().any(|arg| arg == "--benchmark");
    let paths: Vec<PathBuf> = args
        .iter()
        .filter(|arg| arg.as_str() != "--benchmark")
        .map(PathBuf::from)
        .collect();

    let mut state = AppState::new(paths)?;
    let start = std::time::Instant::now();

    if benchmark {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend)?;
        events::render_once(&mut state, &mut terminal)?;
        println!("{}", start.elapsed().as_millis());
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = events::run(&mut state, &mut terminal);

    // Cleanup
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}
