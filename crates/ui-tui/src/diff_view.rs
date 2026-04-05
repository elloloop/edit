use core_diff::{DiffTag, FileDiff};
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};
use ratatui::Frame;

pub fn render_diff(f: &mut Frame, area: Rect, diff: &FileDiff, theme: &Theme) {
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.editor_bg).fg(theme.editor_fg))
        .title(format!(
            " Diff: {} (+{} -{})",
            diff.path, diff.additions, diff.deletions
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let height = inner.height as usize;
    let mut lines_rendered = 0;

    for hunk in &diff.hunks {
        if lines_rendered >= height {
            break;
        }

        // Hunk header
        let header = format!(
            "@@ -{},{} +{},{} @@",
            hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
        );
        let header_style = Style::default()
            .fg(theme.diff_hunk_fg)
            .bg(theme.diff_hunk_bg);
        let header_line = Line::from(Span::styled(
            format!("{header:<width$}", width = inner.width as usize),
            header_style,
        ));
        let y = inner.y + lines_rendered as u16;
        if y < inner.y + inner.height {
            header_line.render(
                Rect::new(inner.x, y, inner.width, 1),
                f.buffer_mut(),
            );
        }
        lines_rendered += 1;

        // Diff lines
        for diff_line in &hunk.lines {
            if lines_rendered >= height {
                break;
            }

            let (prefix, style) = match diff_line.tag {
                DiffTag::Add => (
                    "+",
                    Style::default()
                        .fg(theme.diff_add_fg)
                        .bg(theme.diff_add_bg),
                ),
                DiffTag::Delete => (
                    "-",
                    Style::default()
                        .fg(theme.diff_del_fg)
                        .bg(theme.diff_del_bg),
                ),
                DiffTag::Context => (
                    " ",
                    Style::default()
                        .fg(theme.editor_fg)
                        .bg(theme.editor_bg),
                ),
            };

            let old_ln = diff_line
                .old_lineno
                .map(|n| format!("{n:>4}"))
                .unwrap_or_else(|| "    ".to_string());
            let new_ln = diff_line
                .new_lineno
                .map(|n| format!("{n:>4}"))
                .unwrap_or_else(|| "    ".to_string());

            let text = format!("{old_ln} {new_ln} {prefix}{}", diff_line.content);
            let max_w = inner.width as usize;
            let display: String = text.chars().take(max_w).collect();
            let padded = format!("{display:<width$}", width = max_w);

            let line = Line::from(Span::styled(padded, style));
            let y = inner.y + lines_rendered as u16;
            if y < inner.y + inner.height {
                line.render(
                    Rect::new(inner.x, y, inner.width, 1),
                    f.buffer_mut(),
                );
            }
            lines_rendered += 1;
        }

        // Blank line between hunks
        lines_rendered += 1;
    }
}
