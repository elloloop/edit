use ignore::WalkBuilder;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
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
    pub entries: Vec<FileEntry>,
    pub expanded: HashSet<PathBuf>,
    pub selected: usize,
    git_statuses: std::collections::HashMap<PathBuf, char>,
}

impl FileTree {
    pub fn build(root: &Path) -> anyhow::Result<Self> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let git_statuses = load_git_statuses(&root);

        let mut tree = Self {
            root: root.clone(),
            entries: Vec::new(),
            expanded: HashSet::new(),
            selected: 0,
            git_statuses,
        };

        // Expand root by default
        tree.expanded.insert(root.clone());
        tree.rebuild_entries();
        Ok(tree)
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

    fn rebuild_entries(&mut self) {
        self.entries.clear();
        self.collect_entries(&self.root.clone(), 0);
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
                let status = line.chars().nth(1).unwrap_or(line.chars().next().unwrap_or(' '));
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
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            for path in event.paths {
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

    watcher.watch(root, RecursiveMode::Recursive)?;

    Ok(FileWatcherHandle { _watcher: watcher })
}
