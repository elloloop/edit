#![allow(dead_code)]

use core_terminal::{TerminalLauncher, DEFAULT_SCROLLBACK_LIMIT};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusTarget {
    TerminalPane(u64),
    Sidebar,
    Editor,
    CommandBar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalLayoutNode {
    Leaf {
        pane_id: u64,
    },
    Split {
        split_id: u64,
        axis: SplitAxis,
        ratio_percent: u16,
        first: Box<TerminalLayoutNode>,
        second: Box<TerminalLayoutNode>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPaneState {
    pub id: u64,
    pub launcher: TerminalLauncher,
    pub title: String,
    pub cwd: PathBuf,
    pub scrollback_limit: usize,
}

impl TerminalPaneState {
    fn new(id: u64, launcher: TerminalLauncher, cwd: PathBuf) -> Self {
        Self {
            id,
            launcher: launcher.clone(),
            title: launcher.label().to_string(),
            cwd,
            scrollback_limit: DEFAULT_SCROLLBACK_LIMIT,
        }
    }

    pub fn cwd_label(&self) -> String {
        self.cwd
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| self.cwd.display().to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalPaneLayout {
    pub pane_id: u64,
    pub area: Rect,
}

#[derive(Debug, Clone)]
pub struct TerminalWorkspaceState {
    panes: BTreeMap<u64, TerminalPaneState>,
    layout: TerminalLayoutNode,
    active_pane_id: u64,
    next_pane_id: u64,
    next_split_id: u64,
}

impl TerminalWorkspaceState {
    pub fn new(root_dir: PathBuf) -> Self {
        let initial = TerminalPaneState::new(1, TerminalLauncher::Shell, root_dir);
        let mut panes = BTreeMap::new();
        panes.insert(initial.id, initial);
        Self {
            panes,
            layout: TerminalLayoutNode::Leaf { pane_id: 1 },
            active_pane_id: 1,
            next_pane_id: 2,
            next_split_id: 1,
        }
    }

    pub fn layout(&self) -> &TerminalLayoutNode {
        &self.layout
    }

    pub fn panes(&self) -> &BTreeMap<u64, TerminalPaneState> {
        &self.panes
    }

    pub fn pane(&self, pane_id: u64) -> Option<&TerminalPaneState> {
        self.panes.get(&pane_id)
    }

    pub fn active_pane_id(&self) -> u64 {
        self.active_pane_id
    }

    pub fn select_pane(&mut self, pane_id: u64) -> bool {
        if self.panes.contains_key(&pane_id) {
            self.active_pane_id = pane_id;
            true
        } else {
            false
        }
    }

    pub fn relaunch_active(&mut self, launcher: TerminalLauncher) -> u64 {
        let pane = self
            .panes
            .get_mut(&self.active_pane_id)
            .expect("active pane should always exist");
        pane.launcher = launcher.clone();
        pane.title = launcher.label().to_string();
        pane.id
    }

    pub fn split_active(&mut self, axis: SplitAxis) -> u64 {
        let new_id = self.next_pane_id;
        self.next_pane_id += 1;
        let split_id = self.next_split_id;
        self.next_split_id += 1;
        let active = self
            .panes
            .get(&self.active_pane_id)
            .expect("active pane should always exist")
            .clone();

        let mut new_pane = TerminalPaneState::new(new_id, TerminalLauncher::Shell, active.cwd);
        new_pane.scrollback_limit = active.scrollback_limit;
        self.panes.insert(new_id, new_pane);
        self.layout = split_layout_node(
            self.layout.clone(),
            self.active_pane_id,
            axis,
            split_id,
            new_id,
        );
        self.active_pane_id = new_id;
        new_id
    }

    pub fn close_active(&mut self) -> Option<u64> {
        if self.panes.len() <= 1 {
            return None;
        }

        let removed = self.active_pane_id;
        self.panes.remove(&removed);
        self.layout = close_layout_node(self.layout.clone(), removed)?;
        self.active_pane_id = first_pane_id(&self.layout)?;
        Some(removed)
    }

    pub fn pane_ids_in_display_order(&self) -> Vec<u64> {
        let mut pane_ids = Vec::with_capacity(self.panes.len());
        collect_pane_ids(&self.layout, &mut pane_ids);
        pane_ids
    }

    pub fn focus_next(&mut self) -> u64 {
        self.cycle_focus(true)
    }

    pub fn focus_previous(&mut self) -> u64 {
        self.cycle_focus(false)
    }

    pub fn layout_rects(&self, area: Rect) -> Vec<TerminalPaneLayout> {
        let mut layouts = Vec::with_capacity(self.panes.len());
        collect_layout_rects(&self.layout, area, &mut layouts);
        layouts
    }

    fn cycle_focus(&mut self, forward: bool) -> u64 {
        let pane_ids = self.pane_ids_in_display_order();
        if pane_ids.is_empty() {
            return self.active_pane_id;
        }

        let current_index = pane_ids
            .iter()
            .position(|pane_id| *pane_id == self.active_pane_id)
            .unwrap_or(0);
        let next_index = if forward {
            (current_index + 1) % pane_ids.len()
        } else if current_index == 0 {
            pane_ids.len() - 1
        } else {
            current_index - 1
        };
        self.active_pane_id = pane_ids[next_index];
        self.active_pane_id
    }
}

impl Default for TerminalWorkspaceState {
    fn default() -> Self {
        Self::new(PathBuf::from("."))
    }
}

fn split_layout_node(
    node: TerminalLayoutNode,
    target_pane_id: u64,
    axis: SplitAxis,
    split_id: u64,
    new_pane_id: u64,
) -> TerminalLayoutNode {
    match node {
        TerminalLayoutNode::Leaf { pane_id } if pane_id == target_pane_id => {
            TerminalLayoutNode::Split {
                split_id,
                axis,
                ratio_percent: 50,
                first: Box::new(TerminalLayoutNode::Leaf { pane_id }),
                second: Box::new(TerminalLayoutNode::Leaf {
                    pane_id: new_pane_id,
                }),
            }
        }
        TerminalLayoutNode::Leaf { pane_id } => TerminalLayoutNode::Leaf { pane_id },
        TerminalLayoutNode::Split {
            split_id: current_split_id,
            axis: current_axis,
            ratio_percent,
            first,
            second,
        } => TerminalLayoutNode::Split {
            split_id: current_split_id,
            axis: current_axis,
            ratio_percent,
            first: Box::new(split_layout_node(
                *first,
                target_pane_id,
                axis,
                split_id,
                new_pane_id,
            )),
            second: Box::new(split_layout_node(
                *second,
                target_pane_id,
                axis,
                split_id,
                new_pane_id,
            )),
        },
    }
}

fn close_layout_node(node: TerminalLayoutNode, target_pane_id: u64) -> Option<TerminalLayoutNode> {
    match node {
        TerminalLayoutNode::Leaf { pane_id } if pane_id == target_pane_id => None,
        leaf @ TerminalLayoutNode::Leaf { .. } => Some(leaf),
        TerminalLayoutNode::Split {
            split_id,
            axis,
            ratio_percent,
            first,
            second,
        } => match (
            close_layout_node(*first, target_pane_id),
            close_layout_node(*second, target_pane_id),
        ) {
            (Some(first), Some(second)) => Some(TerminalLayoutNode::Split {
                split_id,
                axis,
                ratio_percent,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (Some(survivor), None) | (None, Some(survivor)) => Some(survivor),
            (None, None) => None,
        },
    }
}

fn first_pane_id(node: &TerminalLayoutNode) -> Option<u64> {
    match node {
        TerminalLayoutNode::Leaf { pane_id } => Some(*pane_id),
        TerminalLayoutNode::Split { first, .. } => first_pane_id(first),
    }
}

fn collect_pane_ids(node: &TerminalLayoutNode, pane_ids: &mut Vec<u64>) {
    match node {
        TerminalLayoutNode::Leaf { pane_id } => pane_ids.push(*pane_id),
        TerminalLayoutNode::Split { first, second, .. } => {
            collect_pane_ids(first, pane_ids);
            collect_pane_ids(second, pane_ids);
        }
    }
}

fn collect_layout_rects(
    node: &TerminalLayoutNode,
    area: Rect,
    layouts: &mut Vec<TerminalPaneLayout>,
) {
    match node {
        TerminalLayoutNode::Leaf { pane_id } => layouts.push(TerminalPaneLayout {
            pane_id: *pane_id,
            area,
        }),
        TerminalLayoutNode::Split {
            axis,
            ratio_percent,
            first,
            second,
            ..
        } => {
            let ratio = (*ratio_percent).clamp(20, 80);
            let split = Layout::default()
                .direction(match axis {
                    SplitAxis::Vertical => Direction::Horizontal,
                    SplitAxis::Horizontal => Direction::Vertical,
                })
                .constraints([
                    Constraint::Percentage(ratio),
                    Constraint::Percentage(100 - ratio),
                ])
                .split(area);
            collect_layout_rects(first, split[0], layouts);
            collect_layout_rects(second, split[1], layouts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn workspace_starts_with_one_shell_pane() {
        let workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        assert_eq!(workspace.panes().len(), 1);
        assert_eq!(workspace.active_pane_id(), 1);
        assert_eq!(
            workspace.panes().get(&1).map(|pane| &pane.launcher),
            Some(&TerminalLauncher::Shell)
        );
        assert_eq!(
            workspace.panes().get(&1).map(|pane| pane.cwd.clone()),
            Some(PathBuf::from("/tmp/workspace"))
        );
    }

    #[test]
    fn splitting_creates_new_active_pane() {
        let mut workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        let pane_id = workspace.split_active(SplitAxis::Horizontal);
        assert_eq!(workspace.panes().len(), 2);
        assert_eq!(workspace.active_pane_id(), pane_id);
        assert!(matches!(
            workspace.layout(),
            TerminalLayoutNode::Split {
                axis: SplitAxis::Horizontal,
                ..
            }
        ));
    }

    #[test]
    fn closing_active_pane_keeps_one_survivor() {
        let mut workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        let second = workspace.split_active(SplitAxis::Vertical);
        assert_eq!(workspace.close_active(), Some(second));
        assert_eq!(workspace.panes().len(), 1);
        assert_eq!(workspace.active_pane_id(), 1);
    }

    #[test]
    fn relaunch_updates_active_pane_metadata() {
        let mut workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        workspace.relaunch_active(TerminalLauncher::Claude);
        let pane = workspace.panes().get(&1).unwrap();
        assert_eq!(pane.title, "Claude");
        assert_eq!(pane.launcher, TerminalLauncher::Claude);
    }

    #[test]
    fn focus_cycle_walks_display_order() {
        let mut workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        let second = workspace.split_active(SplitAxis::Vertical);
        let third = workspace.split_active(SplitAxis::Horizontal);

        assert_eq!(workspace.pane_ids_in_display_order(), vec![1, second, third]);
        assert_eq!(workspace.focus_previous(), second);
        assert_eq!(workspace.focus_next(), third);
        assert_eq!(workspace.focus_next(), 1);
    }

    #[test]
    fn layout_rects_follow_nested_splits() {
        let mut workspace = TerminalWorkspaceState::new(PathBuf::from("/tmp/workspace"));
        workspace.split_active(SplitAxis::Vertical);
        workspace.split_active(SplitAxis::Horizontal);

        let layouts = workspace.layout_rects(Rect::new(0, 0, 120, 40));
        assert_eq!(layouts.len(), 3);
        assert_eq!(layouts[0].area.width, 60);
        assert_eq!(layouts[1].area.width, 60);
        assert_eq!(layouts[1].area.height, 20);
        assert_eq!(layouts[2].area.height, 20);
    }

    #[test]
    fn focus_target_can_point_at_terminal_or_editor_regions() {
        let terminal = FocusTarget::TerminalPane(7);
        let editor = FocusTarget::Editor;
        let sidebar = FocusTarget::Sidebar;
        let command = FocusTarget::CommandBar;
        assert_ne!(terminal, editor);
        assert_ne!(sidebar, command);
    }
}
