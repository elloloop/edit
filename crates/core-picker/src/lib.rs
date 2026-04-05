use ignore::WalkBuilder;
use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::Matcher;
use nucleo::Utf32Str;
use std::path::{Path, PathBuf};

/// A wrapper around PathBuf that implements Display for picker usage.
#[derive(Debug, Clone)]
pub struct PickerPath(pub PathBuf);

impl std::fmt::Display for PickerPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl PickerPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

pub struct Picker<T: Clone> {
    query: String,
    items: Vec<T>,
    display_strings: Vec<String>,
    filtered: Vec<(usize, u32)>, // (index, score)
    selected: usize,
}

impl<T: Clone + ToString> Picker<T> {
    pub fn new(items: Vec<T>) -> Self {
        let display_strings: Vec<String> = items.iter().map(|i| i.to_string()).collect();
        let filtered: Vec<(usize, u32)> = (0..items.len()).map(|i| (i, 0)).collect();
        Self {
            query: String::new(),
            items,
            display_strings,
            filtered,
            selected: 0,
        }
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.refilter();
        self.selected = 0;
    }

    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
        self.refilter();
        self.selected = 0;
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
        self.refilter();
        self.selected = 0;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn selected_item(&self) -> Option<&T> {
        self.filtered
            .get(self.selected)
            .map(|(idx, _)| &self.items[*idx])
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let count = self.filtered.len() as i32;
        let new_sel = self.selected as i32 + delta;
        self.selected = new_sel.rem_euclid(count) as usize;
    }

    pub fn filtered_items(&self) -> Vec<&T> {
        self.filtered.iter().map(|(idx, _)| &self.items[*idx]).collect()
    }

    pub fn filtered_count(&self) -> usize {
        self.filtered.len()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.items.len()).map(|i| (i, 0)).collect();
            return;
        }

        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Smart,
            Normalization::Smart,
            nucleo::pattern::AtomKind::Fuzzy,
        );

        let mut matcher = Matcher::default();
        let mut results: Vec<(usize, u32)> = Vec::new();
        let mut buf = Vec::new();

        for (idx, display) in self.display_strings.iter().enumerate() {
            let haystack = Utf32Str::new(display, &mut buf);
            if let Some(score) = pattern.score(haystack, &mut matcher) {
                results.push((idx, score));
            }
        }

        results.sort_by(|a, b| b.1.cmp(&a.1));
        self.filtered = results;
    }
}

pub fn file_picker(root: &Path) -> Picker<PickerPath> {
    let mut paths = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        if entry.file_type().is_some_and(|ft| ft.is_file()) {
            if let Ok(rel) = entry.path().strip_prefix(root) {
                paths.push(PickerPath(rel.to_path_buf()));
            }
        }
    }

    paths.sort_by(|a, b| a.0.cmp(&b.0));
    Picker::new(paths)
}

#[derive(Debug, Clone)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub shortcut: Option<String>,
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ref shortcut) = self.shortcut {
            write!(f, "{} ({})", self.name, shortcut)
        } else {
            write!(f, "{}", self.name)
        }
    }
}

pub fn command_picker() -> Picker<Command> {
    let commands = vec![
        Command {
            name: "Save".to_string(),
            description: "Save current file".to_string(),
            shortcut: Some("Ctrl-S".to_string()),
        },
        Command {
            name: "Close Tab".to_string(),
            description: "Close current tab".to_string(),
            shortcut: Some("Ctrl-W".to_string()),
        },
        Command {
            name: "Open File".to_string(),
            description: "Open file picker".to_string(),
            shortcut: Some("Ctrl-P".to_string()),
        },
        Command {
            name: "Toggle Sidebar".to_string(),
            description: "Show/hide file tree".to_string(),
            shortcut: Some("Ctrl-B".to_string()),
        },
        Command {
            name: "Toggle Diff".to_string(),
            description: "Show/hide diff view".to_string(),
            shortcut: Some("Ctrl-D".to_string()),
        },
        Command {
            name: "Go to Line".to_string(),
            description: "Jump to a specific line".to_string(),
            shortcut: Some("Ctrl-G".to_string()),
        },
        Command {
            name: "Help".to_string(),
            description: "Show help overlay".to_string(),
            shortcut: Some("?".to_string()),
        },
        Command {
            name: "Quit".to_string(),
            description: "Exit the editor".to_string(),
            shortcut: Some("Ctrl-Q".to_string()),
        },
        Command {
            name: "Next Tab".to_string(),
            description: "Switch to next tab".to_string(),
            shortcut: Some("Ctrl-Tab".to_string()),
        },
        Command {
            name: "Previous Tab".to_string(),
            description: "Switch to previous tab".to_string(),
            shortcut: Some("Ctrl-Shift-Tab".to_string()),
        },
        Command {
            name: "Search".to_string(),
            description: "Search in current file".to_string(),
            shortcut: Some("/".to_string()),
        },
        Command {
            name: "Next Diff Hunk".to_string(),
            description: "Jump to next diff hunk".to_string(),
            shortcut: Some("F8".to_string()),
        },
    ];
    Picker::new(commands)
}
