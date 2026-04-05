use core_picker::Picker;
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use ratatui::Frame;

pub fn render_picker<T: Clone + ToString>(
    f: &mut Frame,
    area: Rect,
    picker: &Picker<T>,
    title: &str,
    theme: &Theme,
) {
    // Clear the area first
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.picker_bg).fg(theme.picker_fg))
        .title(format!(" {title} "));
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    // Query line
    let query_line = Line::from(vec![
        Span::styled(
            " > ",
            Style::default()
                .fg(theme.picker_match)
                .bg(theme.picker_bg),
        ),
        Span::styled(
            picker.query().to_string(),
            Style::default()
                .fg(theme.picker_fg)
                .bg(theme.picker_bg),
        ),
        Span::styled(
            "_",
            Style::default()
                .fg(theme.picker_match)
                .bg(theme.picker_bg),
        ),
    ]);
    query_line.render(
        Rect::new(inner.x, inner.y, inner.width, 1),
        f.buffer_mut(),
    );

    // Count line
    let count_text = format!(
        " {}/{} ",
        picker.filtered_count(),
        picker.filtered_count()
    );
    let count_line = Line::from(Span::styled(
        count_text,
        Style::default()
            .fg(theme.picker_fg)
            .bg(theme.picker_bg),
    ));
    if inner.height >= 2 {
        count_line.render(
            Rect::new(inner.x, inner.y + 1, inner.width, 1),
            f.buffer_mut(),
        );
    }

    // Items
    let items_start = inner.y + 2;
    let items_height = inner.height.saturating_sub(2) as usize;
    let items = picker.filtered_items();
    let selected = picker.selected_index();

    // Compute scroll offset
    let scroll = if selected >= items_height {
        selected - items_height + 1
    } else {
        0
    };

    for (i, item) in items.iter().skip(scroll).take(items_height).enumerate() {
        let y = items_start + i as u16;
        if y >= inner.y + inner.height {
            break;
        }

        let is_selected = scroll + i == selected;
        let (bg, fg) = if is_selected {
            (theme.picker_selected, theme.picker_fg)
        } else {
            (theme.picker_bg, theme.picker_fg)
        };

        let text = item.to_string();
        let max_w = inner.width as usize;
        let display: String = text.chars().take(max_w).collect();
        let padded = format!(" {display:<width$}", width = max_w.saturating_sub(1));

        let line = Line::from(Span::styled(
            padded,
            Style::default().fg(fg).bg(bg),
        ));
        line.render(
            Rect::new(inner.x, y, inner.width, 1),
            f.buffer_mut(),
        );
    }
}
