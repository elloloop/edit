use core_buffer::Buffer;
use core_diff::{ChangedFile, FileDiff};
use core_fs::FileTree;
use core_picker::{Picker, PickerPath};
use core_syntax::Highlighter;
use core_theme::Theme;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
}

pub struct AppState {
    pub buffers: Vec<Buffer>,
    pub active_buffer: usize,
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
    pub quit: bool,
    pub root_dir: PathBuf,
}

impl AppState {
    pub fn new(path: Option<PathBuf>) -> anyhow::Result<Self> {
        let cwd = std::env::current_dir()?;

        let (root_dir, initial_file) = match path {
            Some(p) => {
                let p = if p.is_absolute() {
                    p
                } else {
                    cwd.join(&p)
                };
                if p.is_dir() {
                    (p, None)
                } else {
                    let dir = p.parent().unwrap_or(&cwd).to_path_buf();
                    (dir, Some(p))
                }
            }
            None => (cwd.clone(), None),
        };

        let file_tree = FileTree::build(&root_dir)?;

        let mut buffers = Vec::new();
        let mut highlighters = HashMap::new();

        if let Some(file_path) = initial_file {
            let buf = Buffer::from_file(&file_path)?;
            let lang = buf.language.clone();
            buffers.push(buf);
            if let Some(hl) = Highlighter::new(&lang) {
                highlighters.insert(0, hl);
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

        // Load changed files for diff
        let changed_files = core_diff::changed_files(&root_dir).unwrap_or_default();

        Ok(Self {
            buffers,
            active_buffer: 0,
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
            quit: false,
            root_dir,
        })
    }

    pub fn open_file(&mut self, path: &Path) -> anyhow::Result<()> {
        // Check if already open
        for (i, buf) in self.buffers.iter().enumerate() {
            if buf.path.as_deref() == Some(path) {
                self.active_buffer = i;
                self.file_tree.reveal_path(path);
                return Ok(());
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

        Ok(())
    }

    pub fn close_active_tab(&mut self) {
        if self.buffers.len() <= 1 {
            // Don't close the last buffer, just clear it
            self.buffers[0] = Buffer::from_string("");
            self.highlighters.remove(&0);
            self.active_buffer = 0;
            return;
        }

        self.highlighters.remove(&self.active_buffer);
        self.buffers.remove(self.active_buffer);

        // Re-index highlighters
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
            if let Ok(rel_path) = path.strip_prefix(&self.root_dir) {
                let rel_str = rel_path.to_string_lossy().to_string();
                let old_content =
                    core_diff::git_show_head(&self.root_dir, &rel_str).unwrap_or_default();
                let new_content = buf.content();
                let diff = FileDiff::compute(&old_content, &new_content, &rel_str);
                self.diffs.insert(path.clone(), diff);
            }
        }
    }
}
