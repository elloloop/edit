use ignore::WalkBuilder;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub depth: usize,
    pub git_status: Option<char>,
}

pub struct FileTree {
    pub root: PathBuf,
    pub roots: Vec<PathBuf>,
    pub entries: Vec<FileEntry>,
    pub expanded: HashSet<PathBuf>,
    pub selected: usize,
    git_statuses: std::collections::HashMap<PathBuf, char>,
}

impl FileTree {
    pub fn build(root: &Path) -> anyhow::Result<Self> {
        Self::build_multi(&[root.to_path_buf()], root)
    }

    pub fn build_multi(roots: &[PathBuf], base_root: &Path) -> anyhow::Result<Self> {
        let roots: Vec<PathBuf> = roots
            .iter()
            .map(|root| root.canonicalize().unwrap_or_else(|_| root.to_path_buf()))
            .collect();
        let root = base_root
            .canonicalize()
            .unwrap_or_else(|_| base_root.to_path_buf());
        let git_statuses = load_git_statuses(&root);

        let mut tree = Self {
            root: root.clone(),
            roots: roots.clone(),
            entries: Vec::new(),
            expanded: HashSet::new(),
            selected: 0,
            git_statuses,
        };

        for root in roots {
            tree.expanded.insert(root);
        }
        tree.rebuild_entries();
        Ok(tree)
    }

    pub fn refresh(&mut self) {
        self.git_statuses = load_git_statuses(&self.root);
        let selected_path = self.selected_entry().map(|entry| entry.path.clone());
        self.rebuild_entries();
        if let Some(selected_path) = selected_path {
            if let Some(idx) = self
                .entries
                .iter()
                .position(|entry| entry.path == selected_path)
            {
                self.selected = idx;
            } else if !self.entries.is_empty() {
                self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            }
        }
    }

    pub fn toggle_expand(&mut self, idx: usize) {
        if let Some(entry) = self.entries.get(idx) {
            if entry.is_dir {
                let path = entry.path.clone();
                if self.expanded.contains(&path) {
                    self.expanded.remove(&path);
                } else {
                    self.expanded.insert(path);
                }
                self.rebuild_entries();
            }
        }
    }

    pub fn collapse_selected_or_select_parent(&mut self) -> bool {
        let Some(entry) = self.selected_entry().cloned() else {
            return false;
        };

        if entry.is_dir && self.expanded.contains(&entry.path) {
            self.expanded.remove(&entry.path);
            self.rebuild_entries();
            if let Some(idx) = self.entries.iter().position(|candidate| candidate.path == entry.path) {
                self.selected = idx;
                return true;
            }
            return false;
        }

        self.select_nearest_visible_parent(&entry.path)
    }

    pub fn expand_selected_or_select_child(&mut self) -> bool {
        let Some(entry) = self.selected_entry().cloned() else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }

        if !self.expanded.contains(&entry.path) {
            self.expanded.insert(entry.path.clone());
            self.rebuild_entries();
            if let Some(idx) = self.entries.iter().position(|candidate| candidate.path == entry.path) {
                self.selected = idx;
            }
            return true;
        }

        if let Some(idx) = self
            .entries
            .iter()
            .enumerate()
            .skip(self.selected + 1)
            .take_while(|(_, candidate)| candidate.depth > entry.depth)
            .find_map(|(idx, candidate)| (candidate.depth == entry.depth + 1).then_some(idx))
        {
            self.selected = idx;
            return true;
        }

        false
    }

    pub fn select_first(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.selected = 0;
        true
    }

    pub fn select_last(&mut self) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        self.selected = self.entries.len() - 1;
        true
    }

    pub fn visible_entries(&self) -> Vec<&FileEntry> {
        self.entries.iter().collect()
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.entries.get(self.selected).map(|e| e.path.as_path())
    }

    pub fn selected_entry(&self) -> Option<&FileEntry> {
        self.entries.get(self.selected)
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.entries.is_empty() {
            return;
        }
        let count = self.entries.len() as i32;
        let new_sel = (self.selected as i32 + delta).rem_euclid(count);
        self.selected = new_sel as usize;
    }

    /// Expand all parent directories of the given path and select it in the tree.
    pub fn reveal_path(&mut self, target: &Path) {
        let target = target
            .canonicalize()
            .unwrap_or_else(|_| target.to_path_buf());

        // Expand every ancestor directory from root down to the file's parent
        let mut ancestor = target.parent();
        while let Some(dir) = ancestor {
            if dir == self.root || dir.starts_with(&self.root) {
                self.expanded.insert(dir.to_path_buf());
            }
            if dir == self.root {
                break;
            }
            ancestor = dir.parent();
        }

        self.rebuild_entries();

        // Select the entry matching the target path
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.path == target {
                self.selected = i;
                return;
            }
        }
    }

    fn rebuild_entries(&mut self) {
        self.entries.clear();
        for root in self.roots.clone() {
            if self.roots.len() > 1 {
                let name = root
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.display().to_string());
                self.entries.push(FileEntry {
                    path: root.clone(),
                    name,
                    is_dir: true,
                    depth: 0,
                    git_status: None,
                });
            }
            if self.expanded.contains(&root) {
                self.collect_entries(&root, usize::from(self.roots.len() > 1));
            }
        }
    }

    fn collect_entries(&mut self, dir: &Path, depth: usize) {
        let walker = WalkBuilder::new(dir)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .max_depth(Some(1))
            .sort_by_file_path(|a, b| {
                let a_is_dir = a.is_dir();
                let b_is_dir = b.is_dir();
                if a_is_dir && !b_is_dir {
                    std::cmp::Ordering::Less
                } else if !a_is_dir && b_is_dir {
                    std::cmp::Ordering::Greater
                } else {
                    a.cmp(b)
                }
            })
            .build();

        for entry in walker.flatten() {
            let path = entry.path().to_path_buf();
            if path == dir {
                continue;
            }
            let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let git_status = if let Ok(rel) = path.strip_prefix(&self.root) {
                self.git_statuses.get(rel).copied()
            } else {
                None
            };

            self.entries.push(FileEntry {
                path: path.clone(),
                name,
                is_dir,
                depth,
                git_status,
            });

            if is_dir && self.expanded.contains(&path) {
                self.collect_entries(&path, depth + 1);
            }
        }
    }

    fn select_nearest_visible_parent(&mut self, path: &Path) -> bool {
        let mut parent = path.parent();
        while let Some(candidate) = parent {
            if let Some(idx) = self
                .entries
                .iter()
                .position(|entry| entry.path == candidate)
            {
                self.selected = idx;
                return true;
            }
            parent = candidate.parent();
        }
        false
    }
}

fn load_git_statuses(root: &Path) -> std::collections::HashMap<PathBuf, char> {
    let mut map = std::collections::HashMap::new();
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .current_dir(root)
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if line.len() < 4 {
                    continue;
                }
                let status = line
                    .chars()
                    .nth(1)
                    .unwrap_or(line.chars().next().unwrap_or(' '));
                let status_char = match status {
                    '?' => '?',
                    'M' => 'M',
                    'A' => 'A',
                    'D' => 'D',
                    ' ' => line.chars().next().unwrap_or(' '),
                    c => c,
                };
                let file_path = PathBuf::from(&line[3..]);
                map.insert(file_path, status_char);
            }
        }
    }
    map
}

#[derive(Debug, Clone)]
pub enum FileEvent {
    Modified(PathBuf),
    Created(PathBuf),
    Deleted(PathBuf),
}

pub struct FileWatcherHandle {
    _watcher: RecommendedWatcher,
}

pub fn watch_directory(root: &Path, tx: Sender<FileEvent>) -> anyhow::Result<FileWatcherHandle> {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let callback_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for path in event.paths {
                if should_ignore_watch_path(&callback_root, &path) {
                    continue;
                }
                let file_event = match event.kind {
                    EventKind::Create(_) => Some(FileEvent::Created(path)),
                    EventKind::Modify(_) => Some(FileEvent::Modified(path)),
                    EventKind::Remove(_) => Some(FileEvent::Deleted(path)),
                    _ => None,
                };
                if let Some(fe) = file_event {
                    let _ = tx.send(fe);
                }
            }
        }
    })?;

    watcher.watch(&root, RecursiveMode::Recursive)?;

    Ok(FileWatcherHandle { _watcher: watcher })
}

fn should_ignore_watch_path(root: &Path, path: &Path) -> bool {
    const IGNORED_SEGMENTS: &[&str] = &[".git", "target", "node_modules", ".next", ".turbo"];

    path.strip_prefix(root)
        .ok()
        .into_iter()
        .flat_map(|relative| relative.components())
        .filter_map(|component| match component {
            std::path::Component::Normal(segment) => Some(segment),
            _ => None,
        })
        .any(|segment| IGNORED_SEGMENTS.iter().any(|ignored| segment == OsStr::new(ignored)))
}

#[cfg(test)]
mod tests {
    use super::{should_ignore_watch_path, FileTree};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn left_collapses_selected_directory() {
        let dir = test_dir("collapse-dir");
        fs::create_dir_all(dir.join("nested")).unwrap();

        let mut tree = FileTree::build(&dir).unwrap();
        let nested = dir.join("nested").canonicalize().unwrap();
        tree.reveal_path(&nested);
        tree.expanded.insert(nested.clone());
        tree.refresh();

        assert!(tree.expanded.contains(&nested));
        assert!(tree.collapse_selected_or_select_parent());
        assert!(!tree.expanded.contains(&nested));
        assert_eq!(tree.selected_entry().map(|entry| entry.path.clone()), Some(nested));
    }

    #[test]
    fn left_on_file_selects_parent_directory() {
        let dir = test_dir("file-parent");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let file = dir.join("nested/file.txt");
        fs::write(&file, "hello\n").unwrap();

        let mut tree = FileTree::build(&dir).unwrap();
        tree.reveal_path(&file);

        assert!(tree.collapse_selected_or_select_parent());
        assert_eq!(
            tree.selected_entry().map(|entry| entry.path.clone()),
            Some(dir.join("nested").canonicalize().unwrap())
        );
    }

    #[test]
    fn right_on_expanded_directory_selects_first_child() {
        let dir = test_dir("dir-child");
        fs::create_dir_all(dir.join("nested")).unwrap();
        let file = dir.join("nested/file.txt");
        fs::write(&file, "hello\n").unwrap();

        let mut tree = FileTree::build(&dir).unwrap();
        let nested = dir.join("nested").canonicalize().unwrap();
        tree.reveal_path(&nested);
        assert!(tree.expand_selected_or_select_child());

        assert!(tree.expand_selected_or_select_child());
        assert_eq!(
            tree.selected_entry().map(|entry| entry.path.clone()),
            Some(file.canonicalize().unwrap())
        );
    }

    #[test]
    fn watcher_filter_ignores_generated_paths() {
        let root = PathBuf::from("/tmp/workspace");

        assert!(should_ignore_watch_path(&root, &root.join("target/debug/edit")));
        assert!(should_ignore_watch_path(
            &root,
            &root.join(".git/objects/ab/cdef")
        ));
        assert!(should_ignore_watch_path(
            &root,
            &root.join("website/node_modules/react/index.js")
        ));
        assert!(!should_ignore_watch_path(
            &root,
            &root.join("crates/app/src/main.rs")
        ));
    }

    fn test_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("edit-core-fs-tests-{name}-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
