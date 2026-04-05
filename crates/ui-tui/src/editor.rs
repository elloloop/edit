use core_buffer::Buffer;
use core_syntax::Highlighter;
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;

pub fn render_editor(
    f: &mut Frame,
    area: Rect,
    buffer: &Buffer,
    highlighter: Option<&Highlighter>,
    theme: &Theme,
) {
    // Fill background
    let bg = ratatui::widgets::Block::default()
        .style(Style::default().bg(theme.editor_bg));
    f.render_widget(bg, area);

    let line_count = buffer.line_count();
    let gutter_width = format!("{}", line_count).len().max(3) + 1; // min 4 chars for gutter
    let gutter_w = gutter_width as u16;

    if area.width <= gutter_w + 1 {
        return;
    }

    let editor_width = area.width - gutter_w;
    let height = area.height as usize;

    let visible = buffer.visible_lines(height);
    let source = buffer.content();

    for (i, (line_idx, line_content)) in visible.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.y + area.height {
            break;
        }

        // Gutter (line number)
        let is_cursor_line = *line_idx == buffer.cursor_line;
        let ln_color = if is_cursor_line {
            theme.editor_line_number_active
        } else {
            theme.editor_line_number
        };
        let ln_text = format!("{:>width$} ", line_idx + 1, width = gutter_width - 1);
        let gutter_span = Span::styled(
            ln_text,
            Style::default().fg(ln_color).bg(theme.editor_gutter_bg),
        );
        let gutter_line = Line::from(gutter_span);
        gutter_line.render(
            Rect::new(area.x, y, gutter_w, 1),
            f.buffer_mut(),
        );

        // Editor content with syntax highlighting
        let spans = if let Some(hl) = highlighter {
            let hl_line = hl.highlight_line(&source, *line_idx);
            build_highlighted_spans(line_content, &hl_line.spans, theme)
        } else {
            vec![Span::styled(
                line_content.to_string(),
                Style::default().fg(theme.editor_fg).bg(theme.editor_bg),
            )]
        };

        let content_line = Line::from(spans);
        let content_rect = Rect::new(area.x + gutter_w, y, editor_width, 1);
        content_line.render(content_rect, f.buffer_mut());

        // Render cursor
        if is_cursor_line {
            let cursor_x = area.x + gutter_w + buffer.cursor_col as u16;
            if cursor_x < area.x + area.width {
                let cursor_rect = Rect::new(cursor_x, y, 1, 1);
                let ch = line_content
                    .chars()
                    .nth(buffer.cursor_col)
                    .unwrap_or(' ');
                let cursor_span = Span::styled(
                    ch.to_string(),
                    Style::default()
                        .fg(theme.editor_bg)
                        .bg(theme.editor_cursor),
                );
                Line::from(cursor_span).render(cursor_rect, f.buffer_mut());
            }
        }
    }

    // If fewer lines than height, just leave bg showing
}

fn build_highlighted_spans(
    line: &str,
    hl_spans: &[core_syntax::HighlightSpan],
    theme: &Theme,
) -> Vec<Span<'static>> {
    if hl_spans.is_empty() || line.is_empty() {
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(theme.editor_fg).bg(theme.editor_bg),
        )];
    }

    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let mut result = Vec::new();
    let mut pos = 0;

    for span in hl_spans {
        let start = span.start.min(len);
        let end = span.end.min(len);
        if start >= end {
            continue;
        }

        // Add unstyled text before this span
        if pos < start {
            let text: String = chars[pos..start].iter().collect();
            result.push(Span::styled(
                text,
                Style::default().fg(theme.editor_fg).bg(theme.editor_bg),
            ));
        }

        // Add styled span
        let text: String = chars[start..end].iter().collect();
        let style = theme.style_for_token(&span.token_type).bg(theme.editor_bg);
        result.push(Span::styled(text, style));
        pos = end;
    }

    // Add remaining unstyled text
    if pos < len {
        let text: String = chars[pos..].iter().collect();
        result.push(Span::styled(
            text,
            Style::default().fg(theme.editor_fg).bg(theme.editor_bg),
        ));
    }

    result
}
