use core_terminal::TerminalSnapshot;
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use ratatui::Frame;

#[derive(Debug, Clone)]
pub struct TerminalPaneRender {
    pub pane_id: u64,
    pub area: Rect,
    pub title: String,
    pub cwd_label: String,
    pub agent_status: Option<String>,
    pub snapshot: Option<TerminalSnapshot>,
    pub active: bool,
}

pub fn render_terminal_workspace(f: &mut Frame, panes: &[TerminalPaneRender], theme: &Theme) {
    for pane in panes {
        render_terminal_pane(f, pane, theme);
    }
}

pub fn render_terminal_pane(
    f: &mut Frame,
    pane: &TerminalPaneRender,
    theme: &Theme,
) {
    let max_title_width = pane.area.width.saturating_sub(2) as usize;
    let border = if pane.active {
        theme.command_bar_info_accent
    } else {
        theme.border
    };
    let status = pane
        .agent_status
        .as_deref()
        .or_else(|| {
            pane.snapshot.as_ref().and_then(|snap| {
                if snap.status.is_empty() {
                    None
                } else {
                    Some(snap.status.as_str())
                }
            })
        })
        .unwrap_or("Starting...");
    let body = pane
        .snapshot
        .as_ref()
        .map(|snap| snap.text.clone())
        .unwrap_or_else(|| "\n".to_string());
    let title = fit_title(
        &format!("{} · {} [{}]", pane.title, pane.cwd_label, status),
        max_title_width,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(border).bg(theme.editor_bg))
        .style(Style::default().bg(theme.editor_bg));

    let paragraph = Paragraph::new(Text::from(body))
        .block(block)
        .style(Style::default().fg(theme.editor_fg).bg(theme.editor_bg));

    paragraph.render(pane.area, f.buffer_mut());
}

fn fit_title(title: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut chars: Vec<char> = title.chars().collect();
    if chars.len() <= width {
        return title.to_string();
    }
    if width <= 3 {
        return "·".repeat(width);
    }
    chars.truncate(width - 1);
    let mut compact: String = chars.into_iter().collect();
    compact.push('…');
    compact
}

#[cfg(test)]
mod tests {
    use super::fit_title;

    #[test]
    fn title_is_truncated_for_narrow_terminal_panes() {
        let title = fit_title("Claude · workspace [Running]", 12);
        assert_eq!(title.chars().count(), 12);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn tiny_panes_fall_back_to_compact_title_marks() {
        assert_eq!(fit_title("Shell", 2), "··");
    }
}
