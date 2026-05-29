use core_buffer::Buffer;
use core_diff::ChangedFile;
use core_diff::FileDiff;
use core_fs::FileTree;
use core_picker::{Picker, PickerPath, SearchMatch};
use core_syntax::Highlighter;
use core_theme::Theme;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use std::fmt::Display;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::command_bar::{self, CommandBarState};
use crate::diff_view;
use crate::editor;
use crate::help;
use crate::picker_ui;
use crate::sidebar;
use crate::tabs::{self, TabInfo};

#[allow(clippy::too_many_arguments)]
pub fn render_app<FTerminal, TTouched>(
    f: &mut Frame,
    buffers: &[Buffer],
    active_buffer: usize,
    split_buffers: Option<(usize, usize)>,
    file_tree: &FileTree,
    sidebar_visible: bool,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diff_mode: bool,
    diffs: &HashMap<PathBuf, FileDiff>,
    help_visible: bool,
    file_picker: Option<&Picker<PickerPath>>,
    changed_picker: Option<&Picker<ChangedFile>>,
    grep_picker: Option<&Picker<SearchMatch>>,
    touched_picker: Option<&Picker<TTouched>>,
    command_input: &str,
    status_message: Option<&str>,
    breadcrumb: &str,
    last_search: &str,
    matching_bracket: Option<(usize, usize)>,
    wrap_lines: bool,
    editing: bool,
    touched_paths: &HashSet<PathBuf>,
    conflict_paths: &HashSet<PathBuf>,
    touched_count: usize,
    current_conflict: bool,
    workspace_summary: Option<&str>,
    terminal_workspace_focused: bool,
    sidebar_focused: bool,
    root_terminal_ratio_percent: u16,
    render_terminal_workspace: FTerminal,
) where
    FTerminal: FnOnce(&mut Frame, Rect),
    TTouched: Display + Clone,
{
    let area = f.area();

    // Main vertical layout: tabs | breadcrumb | body | info line | command input
    let main_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // tab bar
            Constraint::Length(1), // breadcrumb
            Constraint::Min(1),    // body
            Constraint::Length(2), // command bar (info + input)
        ])
        .split(area);

    let tab_area = main_layout[0];
    let breadcrumb_area = main_layout[1];
    let body_area = main_layout[2];
    let command_area = main_layout[3];

    // Render tabs
    let conflict_names = conflict_file_names(buffers);
    let tab_infos: Vec<TabInfo> = buffers
        .iter()
        .map(|b| TabInfo {
            name: display_tab_name(b, &conflict_names),
            dirty: b.dirty,
        })
        .collect();
    tabs::render_tabs(f, tab_area, &tab_infos, active_buffer, theme);
    render_breadcrumb(f, breadcrumb_area, breadcrumb, theme);

    let [terminal_area, editor_area] = split_root_workspace(body_area, root_terminal_ratio_percent);
    render_workspace_panel(
        f,
        terminal_area,
        "Terminal Workspace",
        theme,
        terminal_workspace_focused,
        render_terminal_workspace,
    );
    render_workspace_panel(
        f,
        editor_area,
        "Editor Workspace",
        theme,
        !terminal_workspace_focused,
        |f, inner| {
            render_editor_workspace(
                f,
                inner,
                buffers,
                active_buffer,
                split_buffers,
                file_tree,
                sidebar_visible,
                sidebar_focused,
                theme,
                highlighters,
                diff_mode,
                diffs,
                last_search,
                matching_bracket,
                wrap_lines,
                touched_paths,
                conflict_paths,
            );
        },
    );

    // Command bar (info line + input)
    let buf = &buffers[active_buffer];
    let cb_state = CommandBarState {
        input: command_input.to_string(),
        status_message: status_message.map(|s| s.to_string()),
        info_override: workspace_summary.map(|s| s.to_string()),
        file_name: buf.file_name(),
        language: buf.language.clone(),
        cursor_line: buf.cursor_line + 1,
        cursor_col: buf.cursor_col + 1,
        total_lines: buf.line_count(),
        dirty: buf.dirty,
        diff_mode,
        editing,
        split_mode: split_buffers.is_some(),
        touched_files: touched_count,
        external_conflict: current_conflict,
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
    if let Some(picker) = changed_picker {
        let overlay = centered_rect(70, 60, area);
        picker_ui::render_picker(f, overlay, picker, "Changed Files", theme);
    }
    if let Some(picker) = grep_picker {
        let overlay = centered_rect(75, 65, area);
        picker_ui::render_picker(f, overlay, picker, "Search Results", theme);
    }
    if let Some(picker) = touched_picker {
        let overlay = centered_rect(70, 60, area);
        picker_ui::render_picker(
            f,
            overlay,
            picker,
            "Touched Files (Enter open, Ctrl-D diff, Ctrl-R reveal)",
            theme,
        );
    }
}

pub fn split_root_workspace(area: Rect, root_terminal_ratio_percent: u16) -> [Rect; 2] {
    let root_ratio = root_terminal_ratio_percent.clamp(20, 80);
    let root_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(root_ratio),
            Constraint::Percentage(100 - root_ratio),
        ])
        .split(area);
    [root_layout[0], root_layout[1]]
}

pub fn workspace_panel_inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    )
}

fn render_workspace_panel<F>(
    f: &mut Frame,
    area: Rect,
    title: &str,
    theme: &Theme,
    focused: bool,
    render_inner: F,
) where
    F: FnOnce(&mut Frame, Rect),
{
    let border_style = if focused {
        Style::default().fg(theme.command_bar_info_accent)
    } else {
        Style::default().fg(theme.border)
    };
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(border_style)
        .style(Style::default().bg(theme.editor_bg));
    let inner = block.inner(area);
    f.render_widget(block, area);
    render_inner(f, inner);
}

#[allow(clippy::too_many_arguments)]
fn render_editor_workspace(
    f: &mut Frame,
    area: Rect,
    buffers: &[Buffer],
    active_buffer: usize,
    split_buffers: Option<(usize, usize)>,
    file_tree: &FileTree,
    sidebar_visible: bool,
    sidebar_focused: bool,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diff_mode: bool,
    diffs: &HashMap<PathBuf, FileDiff>,
    last_search: &str,
    matching_bracket: Option<(usize, usize)>,
    wrap_lines: bool,
    touched_paths: &HashSet<PathBuf>,
    conflict_paths: &HashSet<PathBuf>,
) {
    if sidebar_visible {
        let body_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(30), // sidebar
                Constraint::Min(1),     // editor
            ])
            .split(area);

        sidebar::render_sidebar(
            f,
            body_layout[0],
            file_tree,
            sidebar_focused,
            touched_paths,
            conflict_paths,
            theme,
        );
        render_editor_or_diff(
            f,
            body_layout[1],
            buffers,
            active_buffer,
            split_buffers,
            theme,
            highlighters,
            diff_mode,
            diffs,
            last_search,
            matching_bracket,
            wrap_lines,
        );
    } else {
        render_editor_or_diff(
            f,
            area,
            buffers,
            active_buffer,
            split_buffers,
            theme,
            highlighters,
            diff_mode,
            diffs,
            last_search,
            matching_bracket,
            wrap_lines,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn render_editor_or_diff(
    f: &mut Frame,
    area: Rect,
    buffers: &[Buffer],
    active_buffer: usize,
    split_buffers: Option<(usize, usize)>,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diff_mode: bool,
    diffs: &HashMap<PathBuf, FileDiff>,
    last_search: &str,
    matching_bracket: Option<(usize, usize)>,
    wrap_lines: bool,
) {
    if let Some((left_idx, right_idx)) = split_buffers {
        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        render_single_editor(
            f,
            layout[0],
            &buffers[left_idx],
            left_idx,
            active_buffer == left_idx,
            theme,
            highlighters,
            diffs,
            last_search,
            matching_bracket,
            wrap_lines,
        );
        render_single_editor(
            f,
            layout[1],
            &buffers[right_idx],
            right_idx,
            active_buffer == right_idx,
            theme,
            highlighters,
            diffs,
            last_search,
            matching_bracket,
            wrap_lines,
        );
        return;
    }

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
                    diffs.get(path),
                    last_search,
                    matching_bracket,
                    wrap_lines,
                );
                diff_view::render_diff(f, layout[1], diff, theme);
                return;
            }
        }
    }

    editor::render_editor(
        f,
        area,
        buf,
        highlighters.get(&active_buffer),
        theme,
        buf.path.as_ref().and_then(|path| diffs.get(path)),
        last_search,
        matching_bracket,
        wrap_lines,
    );
}

#[allow(clippy::too_many_arguments)]
fn render_single_editor(
    f: &mut Frame,
    area: Rect,
    buf: &Buffer,
    buffer_index: usize,
    active: bool,
    theme: &Theme,
    highlighters: &HashMap<usize, Highlighter>,
    diffs: &HashMap<PathBuf, FileDiff>,
    last_search: &str,
    matching_bracket: Option<(usize, usize)>,
    wrap_lines: bool,
) {
    if active {
        let block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(ratatui::style::Style::default().fg(theme.command_bar_info_accent));
        let inner = block.inner(area);
        f.render_widget(block, area);
        editor::render_editor(
            f,
            inner,
            buf,
            highlighters.get(&buffer_index),
            theme,
            buf.path.as_ref().and_then(|path| diffs.get(path)),
            last_search,
            matching_bracket,
            wrap_lines,
        );
    } else {
        editor::render_editor(
            f,
            area,
            buf,
            highlighters.get(&buffer_index),
            theme,
            buf.path.as_ref().and_then(|path| diffs.get(path)),
            last_search,
            matching_bracket,
            wrap_lines,
        );
    }
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

fn render_breadcrumb(f: &mut Frame, area: Rect, breadcrumb: &str, theme: &Theme) {
    let style = ratatui::style::Style::default()
        .fg(theme.command_bar_info_accent)
        .bg(theme.editor_bg);
    let bg = ratatui::widgets::Block::default().style(style);
    f.render_widget(bg, area);
    let text = format!("  {breadcrumb}");
    f.render_widget(ratatui::text::Line::from(text), area);
}

fn conflict_file_names(buffers: &[Buffer]) -> HashSet<String> {
    let mut counts = HashMap::new();
    for buffer in buffers {
        *counts.entry(buffer.file_name()).or_insert(0usize) += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect()
}

fn display_tab_name(buffer: &Buffer, conflict_names: &HashSet<String>) -> String {
    let file_name = buffer.file_name();
    if !conflict_names.contains(&file_name) {
        return file_name;
    }

    if let Some(path) = buffer.path.as_ref() {
        if let Some(parent) = path.parent() {
            if let Some(parent_name) = parent.file_name() {
                return format!("{}/{}", parent_name.to_string_lossy(), file_name);
            }
        }
    }

    file_name
}

#[cfg(test)]
mod tests {
    use super::split_root_workspace;
    use ratatui::layout::Rect;

    #[test]
    fn root_workspace_split_respects_terminal_ratio() {
        let area = Rect::new(0, 0, 100, 40);
        let [terminal, editor] = split_root_workspace(area, 42);
        assert_eq!(terminal.width, 42);
        assert_eq!(editor.width, 58);
        assert_eq!(terminal.height, 40);
        assert_eq!(editor.height, 40);
    }
}
