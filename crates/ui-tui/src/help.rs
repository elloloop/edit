use core_theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};
use ratatui::Frame;

pub fn render_help(f: &mut Frame, area: Rect, theme: &Theme) {
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(theme.picker_bg).fg(theme.picker_fg))
        .title(" Help ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sections: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "Commands (type in command bar, press Enter)",
            vec![
                ("42 or :42", "Go to line 42"),
                ("/pattern", "Search for text"),
                ("Enter (empty)", "Repeat last search"),
                ("n", "Next search result"),
                ("goto <name>", "Jump to function/symbol"),
                ("open or o", "Open file picker"),
                ("save or s", "Save current file"),
                ("close", "Close current tab"),
                ("diff or d", "Toggle diff view"),
                ("sidebar", "Toggle sidebar"),
                ("next / prev", "Switch tabs"),
                ("top / bottom", "Jump to start/end"),
                ("exit or q", "Quit"),
                ("help", "Show this help"),
            ],
        ),
        (
            "Navigation (always active)",
            vec![
                ("Arrow keys", "Move cursor"),
                ("Home / End", "Start / end of line"),
                ("Page Up / Down", "Scroll by page"),
                ("F3 / Shift-F3", "Next / prev search result"),
                ("F8", "Next diff hunk"),
            ],
        ),
        (
            "Shortcuts",
            vec![
                ("Ctrl-S", "Save"),
                ("Ctrl-P", "Open file picker"),
                ("Ctrl-D", "Toggle diff view"),
                ("Ctrl-G", "Go to line (prefills :)"),
                ("Ctrl-B", "Toggle sidebar"),
                ("Ctrl-W", "Close tab"),
                ("Ctrl-Tab / F2", "Next tab"),
                ("Ctrl-Q", "Quit"),
                ("Tab", "Focus sidebar"),
                ("Esc", "Clear command / close overlay"),
            ],
        ),
    ];

    let mut y = inner.y;
    let key_style = Style::default()
        .fg(theme.syntax_keyword)
        .bg(theme.picker_bg)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default()
        .fg(theme.picker_fg)
        .bg(theme.picker_bg);
    let header_style = Style::default()
        .fg(theme.picker_match)
        .bg(theme.picker_bg)
        .add_modifier(Modifier::BOLD);

    for (section_name, bindings) in &sections {
        if y >= inner.y + inner.height {
            break;
        }

        // Section header
        let header = Line::from(Span::styled(format!("  {section_name}"), header_style));
        header.render(Rect::new(inner.x, y, inner.width, 1), f.buffer_mut());
        y += 1;

        for (key, desc) in bindings {
            if y >= inner.y + inner.height {
                break;
            }

            let line = Line::from(vec![
                Span::styled(format!("    {key:<22}"), key_style),
                Span::styled((*desc).to_string(), desc_style),
            ]);
            line.render(
                Rect::new(inner.x, y, inner.width, 1),
                f.buffer_mut(),
            );
            y += 1;
        }

        y += 1; // Blank line between sections
    }
}
