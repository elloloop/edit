use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;

pub struct StatusBarInfo {
    pub file_name: String,
    pub language: String,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub total_lines: usize,
    pub dirty: bool,
    pub message: Option<String>,
    pub search_query: Option<String>,
    pub goto_input: Option<String>,
}

pub fn render_statusbar(f: &mut Frame, area: Rect, state: &StatusBarInfo, theme: &Theme) {
    let style = Style::default()
        .fg(theme.statusbar_fg)
        .bg(theme.statusbar_bg);

    // Fill background
    let bg = ratatui::widgets::Block::default().style(style);
    bg.render(area, f.buffer_mut());

    // Build status text
    let dirty_marker = if state.dirty { " [+]" } else { "" };

    // If we have a search query or goto input, show that prominently
    let left_text = if let Some(ref query) = state.search_query {
        format!(" Search: {query}_")
    } else if let Some(ref input) = state.goto_input {
        format!(" Go to line: {input}_")
    } else if let Some(ref msg) = state.message {
        format!(" {msg}")
    } else {
        format!(" {}{dirty_marker}", state.file_name)
    };

    let right_text = format!(
        "{} | Ln {}, Col {} | {} lines ",
        state.language, state.cursor_line, state.cursor_col, state.total_lines
    );

    let width = area.width as usize;
    let left_len = left_text.len();
    let right_len = right_text.len();

    let mut spans = vec![Span::styled(left_text, style)];

    if left_len + right_len < width {
        let padding = width - left_len - right_len;
        spans.push(Span::styled(" ".repeat(padding), style));
    }
    spans.push(Span::styled(right_text, style));

    let line = Line::from(spans);
    line.render(area, f.buffer_mut());
}
