use core_buffer::Buffer;
use core_diff::FileDiff;
use core_fs::FileTree;
use core_picker::{Picker, PickerPath};
use core_syntax::Highlighter;
use core_theme::Theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::Frame;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::command_bar::{self, CommandBarState};
use crate::diff_view;
use crate::editor;
use crate::help;
use crate::picker_ui;
use crate::sidebar;
use crate::tabs::{self, TabInfo};

#[allow(clippy::too_many_arguments)]
pub fn render_app(
    f: &mut Frame,
    buffers: &[Buffer],
    active_buffer: usize,
    file_tree: &FileTree,
    sidebar_visible: bool,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diff_mode: bool,
    diffs: &HashMap<PathBuf, FileDiff>,
    help_visible: bool,
    file_picker: Option<&Picker<PickerPath>>,
    command_input: &str,
    status_message: Option<&str>,
) {
    let area = f.area();

    // Main vertical layout: tabs | body | info line | command input
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab bar
            Constraint::Min(1),   // body
            Constraint::Length(2), // command bar (info + input)
        ])
        .split(area);

    let tab_area = main_layout[0];
    let body_area = main_layout[1];
    let command_area = main_layout[2];

    // Render tabs
    let tab_infos: Vec<TabInfo> = buffers
        .iter()
        .map(|b| TabInfo {
            name: b.file_name(),
            dirty: b.dirty,
        })
        .collect();
    tabs::render_tabs(f, tab_area, &tab_infos, active_buffer, theme);

    // Body: sidebar (optional) | editor/diff
    if sidebar_visible {
        let body_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(30), // sidebar
                Constraint::Min(1),    // editor
            ])
            .split(body_area);

        sidebar::render_sidebar(f, body_layout[0], file_tree, theme);
        render_editor_or_diff(
            f,
            body_layout[1],
            buffers,
            active_buffer,
            theme,
            highlighters,
            diff_mode,
            diffs,
        );
    } else {
        render_editor_or_diff(
            f,
            body_area,
            buffers,
            active_buffer,
            theme,
            highlighters,
            diff_mode,
            diffs,
        );
    }

    // Command bar (info line + input)
    let buf = &buffers[active_buffer];
    let cb_state = CommandBarState {
        input: command_input.to_string(),
        status_message: status_message.map(|s| s.to_string()),
        file_name: buf.file_name(),
        language: buf.language.clone(),
        cursor_line: buf.cursor_line + 1,
        cursor_col: buf.cursor_col + 1,
        total_lines: buf.line_count(),
        dirty: buf.dirty,
        diff_mode,
    };
    command_bar::render_command_bar(f, command_area, &cb_state, theme);

    // Overlays
    if help_visible {
        let overlay = centered_rect(70, 80, area);
        help::render_help(f, overlay, theme);
    }

    if let Some(picker) = file_picker {
        let overlay = centered_rect(60, 50, area);
        picker_ui::render_picker(f, overlay, picker, "Open File", theme);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_editor_or_diff(
    f: &mut Frame,
    area: Rect,
    buffers: &[Buffer],
    active_buffer: usize,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diff_mode: bool,
    diffs: &HashMap<PathBuf, FileDiff>,
) {
    let buf = &buffers[active_buffer];

    if diff_mode {
        if let Some(path) = &buf.path {
            if let Some(diff) = diffs.get(path) {
                // Split: editor on left, diff on right
                let layout = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area);

                editor::render_editor(
                    f,
                    layout[0],
                    buf,
                    highlighters.get(&active_buffer),
                    theme,
                );
                diff_view::render_diff(f, layout[1], diff, theme);
                return;
            }
        }
    }

    editor::render_editor(f, area, buf, highlighters.get(&active_buffer), theme);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
