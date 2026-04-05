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
        .title(" Help — Keybindings ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let sections: Vec<(&str, Vec<(&str, &str)>)> = vec![
        (
            "Navigation",
            vec![
                ("Arrow keys", "Move cursor"),
                ("Home / End", "Start / end of line"),
                ("Page Up / Down", "Scroll by page"),
                ("Ctrl-G", "Go to line"),
            ],
        ),
        (
            "Editing",
            vec![
                ("Type", "Insert text"),
                ("Backspace", "Delete character before cursor"),
                ("Delete", "Delete character at cursor"),
                ("Enter", "New line"),
                ("Ctrl-S", "Save file"),
            ],
        ),
        (
            "Tabs & Files",
            vec![
                ("Ctrl-P", "Open file picker"),
                ("Ctrl-W", "Close current tab"),
                ("Ctrl-Tab", "Next tab"),
                ("Ctrl-Shift-Tab", "Previous tab"),
                ("Ctrl-B", "Toggle sidebar"),
                ("Enter (sidebar)", "Open file / toggle folder"),
            ],
        ),
        (
            "Search & Diff",
            vec![
                ("/", "Search in file"),
                ("Ctrl-D", "Toggle diff view"),
                ("F8", "Next diff hunk"),
            ],
        ),
        (
            "General",
            vec![
                (":", "Command palette"),
                ("?", "Toggle this help"),
                ("Escape", "Close overlay / cancel"),
                ("Ctrl-Q", "Quit"),
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
