use core_fs::FileTree;
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui::Frame;

pub fn render_sidebar(f: &mut Frame, area: Rect, tree: &FileTree, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.sidebar_bg).fg(theme.sidebar_fg))
        .title(" Files ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let entries = tree.visible_entries();
    let height = inner.height as usize;

    // Compute scroll offset to keep selection visible
    let scroll = if tree.selected >= height {
        tree.selected - height + 1
    } else {
        0
    };

    for (i, entry) in entries.iter().skip(scroll).take(height).enumerate() {
        let y = inner.y + i as u16;
        if y >= inner.y + inner.height {
            break;
        }

        let is_selected = scroll + i == tree.selected;
        let (bg, fg) = if is_selected {
            (theme.sidebar_active_bg, theme.sidebar_active_fg)
        } else {
            (theme.sidebar_bg, theme.sidebar_fg)
        };

        let indent = "  ".repeat(entry.depth);
        let icon = if entry.is_dir { "+" } else { " " };
        let git_marker = entry.git_status.map_or(String::new(), |c| format!(" [{c}]"));

        let label = format!("{indent}{icon} {}{git_marker}", entry.name);
        let max_width = inner.width as usize;
        let display: String = label.chars().take(max_width).collect();

        let style = Style::default().fg(fg).bg(bg);
        let line = Line::from(Span::styled(
            format!("{display:<width$}", width = max_width),
            style,
        ));
        line.render(
            Rect::new(inner.x, y, inner.width, 1),
            f.buffer_mut(),
        );
    }
}
