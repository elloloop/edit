use core_buffer::Buffer;
use core_diff::{ChangedFile, FileDiff};
use core_fs::{FileEvent, FileTree, FileWatcherHandle};
use core_picker::{Picker, PickerPath, SearchMatch};
use core_syntax::Highlighter;
use core_theme::Theme;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Normal,
    Picker,
    Help,
}

#[allow(dead_code)]
pub enum ActivePicker {
    File(Picker<PickerPath>),
    ChangedFiles(Picker<ChangedFile>),
    GrepResults(Picker<SearchMatch>),
}

pub struct AppState {
    pub buffers: Vec<Buffer>,
    pub active_buffer: usize,
    pub split_buffers: Option<(usize, usize)>,
    pub file_tree: FileTree,
    pub sidebar_visible: bool,
    pub sidebar_focused: bool,
    pub theme: Theme,
    pub highlighters: HashMap<usize, Highlighter>,
    pub mode: AppMode,
    pub picker: Option<ActivePicker>,
    pub diff_mode: bool,
    pub diffs: HashMap<PathBuf, FileDiff>,
    #[allow(dead_code)]
    pub changed_files: Vec<ChangedFile>,
    pub help_visible: bool,
    pub status_message: Option<(String, Instant)>,
    pub command_input: String,
    pub last_search: String,
    pub editing: bool,
    pub wrap_lines: bool,
    pub viewport_height: usize,
    pub close_dirty_confirm: bool,
    pub quit: bool,
    pub root_dir: PathBuf,
    // File watching — auto-reload when agents change files on disk
    #[allow(dead_code)]
    file_watcher: Option<FileWatcherHandle>,
    pub file_events: Option<Receiver<FileEvent>>,
}

impl AppState {
    pub fn new(paths: Vec<PathBuf>) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;

        let normalized_paths: Vec<PathBuf> = if paths.is_empty() {
            vec![cwd.clone()]
        } else {
            paths
                .iter()
                .map(|path| {
                    if path.is_absolute() {
                        path.clone()
                    } else {
                        cwd.join(path)
                    }
                })
                .collect()
        };

        let mut roots = Vec::new();
        let mut initial_files = Vec::new();
        for path in normalized_paths {
            if path.is_dir() {
                roots.push(path);
            } else {
                initial_files.push(path.clone());
                roots.push(path.parent().unwrap_or(&cwd).to_path_buf());
            }
        }
        roots.sort();
        roots.dedup();

        let root_dir = common_ancestor(&roots).unwrap_or_else(|| cwd.clone());
        let file_tree = FileTree::build_multi(&roots, &root_dir)?;

        let mut buffers = Vec::new();
        let mut highlighters = HashMap::new();

        for file_path in initial_files {
            let buf = Buffer::from_file(&file_path)?;
            let lang = buf.language.clone();
            buffers.push(buf);
            let idx = buffers.len() - 1;
            if let Some(hl) = Highlighter::new(&lang) {
                highlighters.insert(idx, hl);
            }
        }

        // Always have at least one buffer
        if buffers.is_empty() {
            buffers.push(Buffer::from_string(""));
        }

        // Parse all buffers for syntax highlighting
        for (idx, hl) in highlighters.iter_mut() {
            let content = buffers[*idx].content();
            hl.parse(&content);
        }

        if let Some(path) = buffers.first().and_then(|buffer| buffer.path.as_deref()) {
            let mut file_tree = file_tree;
            file_tree.reveal_path(path);

            // Load changed files for diff
            let changed_files = core_diff::changed_files(&root_dir).unwrap_or_default();

            // Start file watcher — watches for changes made by agents/editors
            let (tx, rx) = std::sync::mpsc::channel();
            let watcher = core_fs::watch_directory(&root_dir, tx).ok();

            return Ok(Self {
                buffers,
                active_buffer: 0,
                split_buffers: None,
                file_tree,
                sidebar_visible: true,
                sidebar_focused: false,
                theme: Theme::dark_plus(),
                highlighters,
                mode: AppMode::Normal,
                picker: None,
                diff_mode: false,
                diffs: HashMap::new(),
                changed_files,
                help_visible: false,
                status_message: None,
                command_input: String::new(),
                last_search: String::new(),
                editing: false,
                wrap_lines: false,
                viewport_height: 0,
                close_dirty_confirm: false,
                quit: false,
                root_dir,
                file_watcher: watcher,
                file_events: Some(rx),
            });
        }

        // Load changed files for diff
        let changed_files = core_diff::changed_files(&root_dir).unwrap_or_default();

        // Start file watcher — watches for changes made by agents/editors
        let (tx, rx) = std::sync::mpsc::channel();
        let watcher = core_fs::watch_directory(&root_dir, tx).ok();

        Ok(Self {
            buffers,
            active_buffer: 0,
            split_buffers: None,
            file_tree,
            sidebar_visible: true,
            sidebar_focused: false,
            theme: Theme::dark_plus(),
            highlighters,
            mode: AppMode::Normal,
            picker: None,
            diff_mode: false,
            diffs: HashMap::new(),
            changed_files,
            help_visible: false,
            status_message: None,
            command_input: String::new(),
            last_search: String::new(),
            editing: false,
            wrap_lines: false,
            viewport_height: 0,
            close_dirty_confirm: false,
            quit: false,
            root_dir,
            file_watcher: watcher,
            file_events: Some(rx),
        })
    }

    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<()> {
        self.open_file_with_index(path).map(|_| ())
    }

    pub fn open_file_with_index(&mut self, path: &Path) -> anyhow::Result<usize> {
        // Check if already open
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path) {
                self.active_buffer = i;
                self.file_tree.reveal_path(path);
                return Ok(i);
            }
        }

        let buf = Buffer::from_file(path)?;
        let lang = buf.language.clone();
        self.buffers.push(buf);
        let idx = self.buffers.len() - 1;
        self.active_buffer = idx;

        if let Some(mut hl) = Highlighter::new(&lang) {
            let content = self.buffers[idx].content();
            hl.parse(&content);
            self.highlighters.insert(idx, hl);
        }

        // Reveal file in sidebar — expand parent folders and select it
        self.file_tree.reveal_path(path);

        Ok(idx)
    }

    pub fn close_active_tab(&mut self) {
        let removed = self.active_buffer;
        if self.buffers.len() <= 1 {
            self.buffers[0] = Buffer::from_string("");
            self.highlighters.remove(&0);
            self.active_buffer = 0;
            self.close_dirty_confirm = false;
            return;
        }

        self.highlighters.remove(&self.active_buffer);
        self.buffers.remove(self.active_buffer);

        let mut new_highlighters = HashMap::new();
        for (idx, hl) in self.highlighters.drain() {
            let new_idx = if idx > self.active_buffer {
                idx - 1
            } else {
                idx
            };
            new_highlighters.insert(new_idx, hl);
        }
        self.highlighters = new_highlighters;

        if self.active_buffer >= self.buffers.len() {
            self.active_buffer = self.buffers.len() - 1;
        }
        self.close_dirty_confirm = false;
        if let Some((left, right)) = self.split_buffers {
            self.split_buffers = remap_split_indices(left, right, removed);
        }
    }

    #[allow(dead_code)]
    pub fn reparse_current_buffer(&mut self) {
        let idx = self.active_buffer;
        let content = self.buffers[idx].content();
        if let Some(hl) = self.highlighters.get_mut(&idx) {
            hl.parse(&content);
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status_message = Some((msg.to_string(), Instant::now()));
    }

    #[allow(dead_code)]
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.active_buffer]
    }

    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.active_buffer]
    }

    pub fn compute_diff_for_current(&mut self) {
        let buf = &self.buffers[self.active_buffer];
        if let Some(ref path) = buf.path.clone() {
            self.compute_diff_for_path(path);
        }
    }

    /// Reload a buffer from disk if it's open and not dirty.
    /// Called when the file watcher detects external changes (e.g., from AI agents).
    pub fn reload_if_open(&mut self, path: &Path) {
        let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        // Find the buffer
        let idx = self.buffers.iter().position(|buf| {
            buf.path.as_ref().map_or(false, |p| {
                p.canonicalize().unwrap_or_else(|_| p.clone()) == canon
            })
        });

        let Some(idx) = idx else { return };

        // Don't overwrite unsaved user edits
        if self.buffers[idx].dirty {
            return;
        }

        // Read new content
        let bytes = match std::fs::read(&canon) {
            Ok(bytes) => bytes,
            Err(_) => return,
        };
        let is_binary = bytes.contains(&0);
        let content = String::from_utf8_lossy(&bytes).into_owned();

        // Skip if content hasn't actually changed
        if content == self.buffers[idx].content() {
            return;
        }

        let file_name = self.buffers[idx].file_name();
        self.buffers[idx].reload(&content);
        self.buffers[idx].is_binary = is_binary;

        // Reparse syntax highlighting
        if let Some(hl) = self.highlighters.get_mut(&idx) {
            hl.parse(&content);
        }

        if let Some(path) = self.buffers[idx].path.clone() {
            self.compute_diff_for_path(&path);
        }

        self.refresh_workspace();
        self.set_status(&format!("Reloaded: {file_name}"));
    }

    pub fn compute_diff_for_path(&mut self, path: &Path) {
        if let Ok(rel_path) = path.strip_prefix(&self.root_dir) {
            let rel_str = rel_path.to_string_lossy().to_string();
            let old_content =
                core_diff::git_show_head(&self.root_dir, &rel_str).unwrap_or_default();
            let new_content = self
                .buffers
                .iter()
                .find(|buf| buf.path.as_deref() == Some(path))
                .map(|buf| buf.content())
                .unwrap_or_else(|| String::new());
            let diff = FileDiff::compute(&old_content, &new_content, &rel_str);
            self.diffs.insert(path.to_path_buf(), diff);
        }
    }

    pub fn refresh_workspace(&mut self) {
        self.file_tree.refresh();
        self.changed_files = core_diff::changed_files(&self.root_dir).unwrap_or_default();
    }

    pub fn enter_split(&mut self, left: usize, right: usize) {
        self.split_buffers = Some((left, right));
        self.active_buffer = left;
    }

    pub fn exit_split(&mut self) {
        self.split_buffers = None;
    }

    pub fn toggle_split_focus(&mut self) {
        if let Some((left, right)) = self.split_buffers {
            self.active_buffer = if self.active_buffer == left {
                right
            } else {
                left
            };
        }
    }

    pub fn breadcrumb(&self) -> String {
        let Some(path) = self.buffers[self.active_buffer].path.as_ref() else {
            return "[untitled]".to_string();
        };
        path.strip_prefix(&self.root_dir)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    pub fn matching_bracket(&self) -> Option<(usize, usize)> {
        let buf = &self.buffers[self.active_buffer];
        let line = buf.get_line(buf.cursor_line)?;
        let chars: Vec<char> = line.chars().collect();

        for &idx in &[buf.cursor_col, buf.cursor_col.saturating_sub(1)] {
            let Some(&ch) = chars.get(idx) else { continue };
            let (open, close, direction) = match ch {
                '(' => ('(', ')', 1isize),
                '[' => ('[', ']', 1isize),
                '{' => ('{', '}', 1isize),
                ')' => ('(', ')', -1isize),
                ']' => ('[', ']', -1isize),
                '}' => ('{', '}', -1isize),
                _ => continue,
            };
            let mut depth = 0isize;
            let mut pos = idx as isize;
            while pos >= 0 && (pos as usize) < chars.len() {
                let current = chars[pos as usize];
                if current == open {
                    depth += if direction > 0 { 1 } else { -1 };
                } else if current == close {
                    depth += if direction > 0 { -1 } else { 1 };
                }
                if depth == 0 && pos as usize != idx {
                    return Some((buf.cursor_line, pos as usize));
                }
                pos += direction;
            }
        }

        None
    }
}

fn common_ancestor(paths: &[PathBuf]) -> Option<PathBuf> {
    let mut components: Vec<_> = paths.first()?.components().collect();
    for path in &paths[1..] {
        let path_components: Vec<_> = path.components().collect();
        let shared = components
            .iter()
            .zip(path_components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        components.truncate(shared);
    }
    if components.is_empty() {
        return None;
    }
    let mut ancestor = PathBuf::new();
    for component in components {
        ancestor.push(component.as_os_str());
    }
    Some(ancestor)
}

fn remap_split_indices(left: usize, right: usize, removed: usize) -> Option<(usize, usize)> {
    let remap = |idx: usize| -> Option<usize> {
        if idx == removed {
            None
        } else if idx > removed {
            Some(idx - 1)
        } else {
            Some(idx)
        }
    };

    let left = remap(left)?;
    let right = remap(right)?;
    if left == right {
        None
    } else {
        Some((left, right))
    }
}

#[cfg(test)]
mod tests {
    use super::{common_ancestor, remap_split_indices};
    use std::path::PathBuf;

    #[test]
    fn common_ancestor_handles_multiple_roots() {
        let roots = vec![
            PathBuf::from("/tmp/work/a"),
            PathBuf::from("/tmp/work/b"),
            PathBuf::from("/tmp/work/c/nested"),
        ];
        assert_eq!(common_ancestor(&roots), Some(PathBuf::from("/tmp/work")));
    }

    #[test]
    fn split_indices_remap_when_tab_closes() {
        assert_eq!(remap_split_indices(1, 3, 0), Some((0, 2)));
        assert_eq!(remap_split_indices(1, 3, 1), None);
        assert_eq!(remap_split_indices(2, 3, 2), None);
    }
}
