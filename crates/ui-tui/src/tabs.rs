use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;

pub struct TabInfo {
    pub name: String,
    pub dirty: bool,
}

pub fn render_tabs(f: &mut Frame, area: Rect, tabs: &[TabInfo], active: usize, theme: &Theme) {
    let mut spans = Vec::new();

    for (i, tab) in tabs.iter().enumerate() {
        let (bg, fg) = if i == active {
            (theme.tab_active_bg, theme.tab_active_fg)
        } else {
            (theme.tab_bg, theme.tab_fg)
        };

        let dirty_marker = if tab.dirty { " *" } else { "" };
        let label = format!(" {}{dirty_marker} ", tab.name);

        spans.push(Span::styled(label, Style::default().fg(fg).bg(bg)));

        if i < tabs.len() - 1 {
            spans.push(Span::styled(
                "|",
                Style::default().fg(theme.border).bg(theme.tab_bg),
            ));
        }
    }

    // Fill remainder
    let line = Line::from(spans);
    let bg_style = Style::default().bg(theme.tab_bg);

    // Render background first
    let bg_block = ratatui::widgets::Block::default().style(bg_style);
    bg_block.render(area, f.buffer_mut());

    // Render tabs line
    f.render_widget(line, area);
}
