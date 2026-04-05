use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;

pub struct CommandBarState {
    pub input: String,
    pub status_message: Option<String>,
    pub file_name: String,
    pub language: String,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub total_lines: usize,
    pub dirty: bool,
    pub diff_mode: bool,
}

/// Renders the bottom area: info line + command input line.
/// Expects `area` to be exactly 2 rows tall.
pub fn render_command_bar(f: &mut Frame, area: Rect, state: &CommandBarState, theme: &Theme) {
    if area.height < 2 {
        return;
    }

    let info_area = Rect::new(area.x, area.y, area.width, 1);
    let input_area = Rect::new(area.x, area.y + 1, area.width, 1);

    render_info_line(f, info_area, state, theme);
    render_input_line(f, input_area, state, theme);
}

fn render_info_line(f: &mut Frame, area: Rect, state: &CommandBarState, theme: &Theme) {
    let style = Style::default()
        .fg(theme.command_bar_info_fg)
        .bg(theme.command_bar_info_bg);

    // Fill background
    let bg = ratatui::widgets::Block::default().style(style);
    bg.render(area, f.buffer_mut());

    let dirty_marker = if state.dirty { " [+]" } else { "" };
    let diff_marker = if state.diff_mode { "  diff" } else { "" };

    let accent = Style::default()
        .fg(theme.command_bar_info_accent)
        .bg(theme.command_bar_info_bg);

    let sep = Span::styled("  ", style);

    let spans = vec![
        Span::styled("  ", style),
        Span::styled(&state.file_name, accent),
        Span::styled(dirty_marker, Style::default().fg(theme.tab_dirty).bg(theme.command_bar_info_bg)),
        sep.clone(),
        Span::styled(&state.language, style),
        sep.clone(),
        Span::styled(
            format!("Ln {}, Col {}", state.cursor_line, state.cursor_col),
            style,
        ),
        sep.clone(),
        Span::styled(format!("{} lines", state.total_lines), style),
        Span::styled(diff_marker, Style::default().fg(theme.command_bar_info_accent).bg(theme.command_bar_info_bg)),
    ];

    let line = Line::from(spans);
    line.render(area, f.buffer_mut());
}

fn render_input_line(f: &mut Frame, area: Rect, state: &CommandBarState, theme: &Theme) {
    let style = Style::default()
        .fg(theme.command_bar_fg)
        .bg(theme.command_bar_bg);

    // Fill background
    let bg = ratatui::widgets::Block::default().style(style);
    bg.render(area, f.buffer_mut());

    let prompt_style = Style::default()
        .fg(theme.command_bar_prompt)
        .bg(theme.command_bar_bg);

    let placeholder_style = Style::default()
        .fg(theme.command_bar_placeholder)
        .bg(theme.command_bar_bg);

    let cursor_style = Style::default()
        .fg(theme.command_bar_bg)
        .bg(theme.command_bar_fg);

    if state.input.is_empty() {
        // Show status message or placeholder
        let display_text = if let Some(ref msg) = state.status_message {
            msg.clone()
        } else {
            String::new()
        };

        let spans = if display_text.is_empty() {
            vec![
                Span::styled("  \u{276f} ", prompt_style),
                Span::styled(
                    "Type a command... (help for keybindings)",
                    placeholder_style,
                ),
            ]
        } else {
            vec![
                Span::styled("  \u{276f} ", prompt_style),
                Span::styled(display_text, style),
            ]
        };

        let line = Line::from(spans);
        line.render(area, f.buffer_mut());
    } else {
        // Show input with cursor
        let spans = vec![
            Span::styled("  \u{276f} ", prompt_style),
            Span::styled(&state.input, style),
            Span::styled(" ", cursor_style),
        ];

        let line = Line::from(spans);
        line.render(area, f.buffer_mut());
    }
}
