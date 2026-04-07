use core_buffer::Buffer;
use core_diff::{DiffTag, FileDiff};
use core_syntax::Highlighter;
use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui::Frame;
use std::collections::HashMap;

pub fn render_editor(
    f: &mut Frame,
    area: Rect,
    buffer: &Buffer,
    highlighter: Option<&Highlighter>,
    theme: &Theme,
    diff: Option<&FileDiff>,
    search_query: &str,
    matching_bracket: Option<(usize, usize)>,
    wrap_lines: bool,
) {
    // Fill background
    let bg = ratatui::widgets::Block::default().style(Style::default().bg(theme.editor_bg));
    f.render_widget(bg, area);

    if buffer.is_binary {
        let line = Line::from(Span::styled(
            "Binary file preview disabled",
            Style::default()
                .fg(theme.command_bar_placeholder)
                .bg(theme.editor_bg),
        ));
        line.render(area, f.buffer_mut());
        return;
    }

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
    let diff_markers = build_diff_markers(diff);
    let mut rendered_rows = 0usize;

    for (line_idx, line_content) in visible.iter() {
        let wrapped_lines = if wrap_lines && editor_width > 0 {
            wrap_line(line_content, editor_width as usize)
        } else {
            vec![line_content.clone()]
        };

        for (segment_idx, segment) in wrapped_lines.iter().enumerate() {
            if rendered_rows >= height {
                break;
            }
            let y = area.y + rendered_rows as u16;
            if y >= area.y + area.height {
                break;
            }

            let is_cursor_line = *line_idx == buffer.cursor_line;
            let ln_color = if is_cursor_line {
                theme.editor_line_number_active
            } else {
                theme.editor_line_number
            };
            let diff_marker = if segment_idx == 0 {
                match diff_markers.get(line_idx) {
                    Some(DiffTag::Add) => "+",
                    Some(DiffTag::Delete) => "-",
                    _ => " ",
                }
            } else {
                " "
            };
            let ln_text = if segment_idx == 0 {
                format!(
                    "{:>width$}{diff_marker}",
                    line_idx + 1,
                    width = gutter_width - 1
                )
            } else {
                format!("{:>width$} ", "", width = gutter_width - 1)
            };
            let gutter_span = Span::styled(
                ln_text,
                Style::default().fg(ln_color).bg(if is_cursor_line {
                    theme.editor_selection
                } else {
                    theme.editor_gutter_bg
                }),
            );
            let gutter_line = Line::from(gutter_span);
            gutter_line.render(Rect::new(area.x, y, gutter_w, 1), f.buffer_mut());

            let spans = if let Some(hl) = highlighter {
                let hl_line = hl.highlight_line(&source, *line_idx);
                build_highlighted_spans(
                    line_content,
                    segment,
                    segment_idx,
                    editor_width as usize,
                    &hl_line.spans,
                    theme,
                    search_query,
                    is_cursor_line,
                    matching_bracket
                        .filter(|(line, _)| *line == *line_idx)
                        .map(|(_, col)| col),
                )
            } else {
                vec![Span::styled(
                    segment.to_string(),
                    default_line_style(theme, is_cursor_line),
                )]
            };

            let content_line = Line::from(spans);
            let content_rect = Rect::new(area.x + gutter_w, y, editor_width, 1);
            content_line.render(content_rect, f.buffer_mut());

            if is_cursor_line {
                let segment_start = segment_idx * editor_width as usize;
                let segment_end = segment_start + segment.chars().count();
                if buffer.cursor_col >= segment_start && buffer.cursor_col <= segment_end {
                    let cursor_x = area.x + gutter_w + (buffer.cursor_col - segment_start) as u16;
                    if cursor_x < area.x + area.width {
                        let cursor_rect = Rect::new(cursor_x, y, 1, 1);
                        let ch = line_content.chars().nth(buffer.cursor_col).unwrap_or(' ');
                        let cursor_span = Span::styled(
                            ch.to_string(),
                            Style::default().fg(theme.editor_bg).bg(theme.editor_cursor),
                        );
                        Line::from(cursor_span).render(cursor_rect, f.buffer_mut());
                    }
                }
            }
            rendered_rows += 1;
        }
    }

    // If fewer lines than height, just leave bg showing
}

fn build_highlighted_spans(
    line: &str,
    segment: &str,
    segment_idx: usize,
    wrap_width: usize,
    hl_spans: &[core_syntax::HighlightSpan],
    theme: &Theme,
    search_query: &str,
    is_cursor_line: bool,
    matching_bracket_col: Option<usize>,
) -> Vec<Span<'static>> {
    if hl_spans.is_empty() || line.is_empty() {
        return vec![Span::styled(
            segment.to_string(),
            default_line_style(theme, is_cursor_line),
        )];
    }

    let chars: Vec<char> = line.chars().collect();
    let len = chars.len();
    let segment_start = segment_idx * wrap_width;
    let segment_end = (segment_start + segment.chars().count()).min(len);
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
            push_segment_text(
                &mut result,
                &chars,
                pos,
                start,
                segment_start,
                segment_end,
                default_line_style(theme, is_cursor_line),
                search_query,
                theme,
                matching_bracket_col,
            );
        }

        // Add styled span
        let style = theme
            .style_for_token(&span.token_type)
            .bg(if is_cursor_line {
                theme.editor_selection
            } else {
                theme.editor_bg
            });
        push_segment_text(
            &mut result,
            &chars,
            start,
            end,
            segment_start,
            segment_end,
            style,
            search_query,
            theme,
            matching_bracket_col,
        );
        pos = end;
    }

    // Add remaining unstyled text
    if pos < len {
        push_segment_text(
            &mut result,
            &chars,
            pos,
            len,
            segment_start,
            segment_end,
            default_line_style(theme, is_cursor_line),
            search_query,
            theme,
            matching_bracket_col,
        );
    }

    result
}

fn push_segment_text(
    result: &mut Vec<Span<'static>>,
    chars: &[char],
    start: usize,
    end: usize,
    segment_start: usize,
    segment_end: usize,
    style: Style,
    search_query: &str,
    theme: &Theme,
    matching_bracket_col: Option<usize>,
) {
    let slice_start = start.max(segment_start);
    let slice_end = end.min(segment_end);
    if slice_start >= slice_end {
        return;
    }

    if search_query.is_empty() && matching_bracket_col.is_none() {
        let text: String = chars[slice_start..slice_end].iter().collect();
        result.push(Span::styled(text, style));
        return;
    }

    for idx in slice_start..slice_end {
        let ch = chars[idx];
        let mut ch_style = style;
        if let Some(bracket_col) = matching_bracket_col {
            if idx == bracket_col {
                ch_style = ch_style.bg(theme.diff_hunk_bg).fg(theme.diff_hunk_fg);
            }
        }
        if !search_query.is_empty() {
            let line_text: String = chars.iter().collect();
            if matches_search_at(&line_text, idx, search_query) {
                ch_style = ch_style
                    .bg(theme.command_bar_info_accent)
                    .fg(theme.editor_bg);
            }
        }
        result.push(Span::styled(ch.to_string(), ch_style));
    }
}

fn build_diff_markers(diff: Option<&FileDiff>) -> HashMap<usize, DiffTag> {
    let mut markers = HashMap::new();
    let Some(diff) = diff else { return markers };
    for hunk in &diff.hunks {
        for line in &hunk.lines {
            if let Some(new_lineno) = line.new_lineno {
                markers.insert(new_lineno.saturating_sub(1), line.tag);
            }
        }
    }
    markers
}

fn default_line_style(theme: &Theme, is_cursor_line: bool) -> Style {
    Style::default().fg(theme.editor_fg).bg(if is_cursor_line {
        theme.editor_selection
    } else {
        theme.editor_bg
    })
}

fn wrap_line(line: &str, width: usize) -> Vec<String> {
    if width == 0 || line.is_empty() {
        return vec![line.to_string()];
    }
    let chars: Vec<char> = line.chars().collect();
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

fn matches_search_at(line: &str, char_idx: usize, query: &str) -> bool {
    if query.is_empty() {
        return false;
    }
    let line_chars: Vec<char> = line.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();
    if char_idx + query_chars.len() > line_chars.len() {
        return false;
    }
    line_chars[char_idx..char_idx + query_chars.len()] == query_chars[..]
}
