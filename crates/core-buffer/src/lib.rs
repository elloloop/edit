use ropey::Rope;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

#[derive(Debug, Clone)]
pub struct Selection {
    pub start_line: usize,
    pub start_col: usize,
    pub end_line: usize,
    pub end_col: usize,
}

pub struct Buffer {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    pub language: String,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_top: usize,
    pub scroll_left: usize,
    pub selection: Option<Selection>,
}

impl Buffer {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let language = detect_language(path);
        Ok(Self {
            rope: Rope::from_str(&content),
            path: Some(path.to_path_buf()),
            dirty: false,
            language,
            cursor_line: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_left: 0,
            selection: None,
        })
    }

    pub fn from_string(content: &str) -> Self {
        Self {
            rope: Rope::from_str(content),
            path: None,
            dirty: false,
            language: String::from("text"),
            cursor_line: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_left: 0,
            selection: None,
        }
    }

    pub fn empty() -> Self {
        Self::from_string("")
    }

    pub fn save(&mut self) -> anyhow::Result<()> {
        if let Some(ref path) = self.path {
            let content = self.rope.to_string();
            fs::write(path, content)?;
            self.dirty = false;
            Ok(())
        } else {
            anyhow::bail!("No file path set for this buffer")
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        let idx = self.cursor_byte_offset();
        let char_idx = self.rope.byte_to_char(idx);
        self.rope.insert_char(char_idx, ch);
        if ch == '\n' {
            self.cursor_line += 1;
            self.cursor_col = 0;
        } else {
            self.cursor_col += 1;
        }
        self.dirty = true;
    }

    pub fn delete_char(&mut self) {
        let line_len = self.current_line_len();
        if self.cursor_col < line_len {
            let idx = self.cursor_byte_offset();
            let char_idx = self.rope.byte_to_char(idx);
            if char_idx < self.rope.len_chars() {
                self.rope.remove(char_idx..char_idx + 1);
                self.dirty = true;
            }
        } else if self.cursor_line + 1 < self.line_count() {
            // Join with next line
            let idx = self.cursor_byte_offset();
            let char_idx = self.rope.byte_to_char(idx);
            if char_idx < self.rope.len_chars() {
                self.rope.remove(char_idx..char_idx + 1);
                self.dirty = true;
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            self.delete_char();
        } else if self.cursor_line > 0 {
            let prev_line_len = self.line_len(self.cursor_line - 1);
            self.cursor_line -= 1;
            self.cursor_col = prev_line_len;
            self.delete_char();
        }
    }

    pub fn new_line(&mut self) {
        self.insert_char('\n');
    }

    pub fn move_cursor(&mut self, direction: Direction, count: usize) {
        for _ in 0..count {
            match direction {
                Direction::Up => {
                    if self.cursor_line > 0 {
                        self.cursor_line -= 1;
                        self.clamp_cursor_col();
                    }
                }
                Direction::Down => {
                    if self.cursor_line + 1 < self.line_count() {
                        self.cursor_line += 1;
                        self.clamp_cursor_col();
                    }
                }
                Direction::Left => {
                    if self.cursor_col > 0 {
                        self.cursor_col -= 1;
                    } else if self.cursor_line > 0 {
                        self.cursor_line -= 1;
                        self.cursor_col = self.current_line_len();
                    }
                }
                Direction::Right => {
                    let line_len = self.current_line_len();
                    if self.cursor_col < line_len {
                        self.cursor_col += 1;
                    } else if self.cursor_line + 1 < self.line_count() {
                        self.cursor_line += 1;
                        self.cursor_col = 0;
                    }
                }
                Direction::Home => {
                    self.cursor_col = 0;
                }
                Direction::End => {
                    self.cursor_col = self.current_line_len();
                }
                Direction::PageUp => {
                    self.cursor_line = self.cursor_line.saturating_sub(30);
                    self.clamp_cursor_col();
                }
                Direction::PageDown => {
                    self.cursor_line = (self.cursor_line + 30).min(self.line_count().saturating_sub(1));
                    self.clamp_cursor_col();
                }
            }
        }
    }

    pub fn go_to_line(&mut self, line: usize) {
        let target = line.min(self.line_count().saturating_sub(1));
        self.cursor_line = target;
        self.clamp_cursor_col();
    }

    pub fn line_count(&self) -> usize {
        self.rope.len_lines().max(1)
    }

    pub fn get_line(&self, idx: usize) -> Option<String> {
        if idx < self.rope.len_lines() {
            let line = self.rope.line(idx);
            let s = line.to_string();
            // Strip trailing newline for display
            Some(s.trim_end_matches('\n').trim_end_matches('\r').to_string())
        } else {
            None
        }
    }

    pub fn cursor_byte_offset(&self) -> usize {
        if self.rope.len_chars() == 0 {
            return 0;
        }
        let line_start = self.rope.line_to_byte(self.cursor_line);
        let line = self.rope.line(self.cursor_line);
        let line_len = line.len_chars();
        let col = self.cursor_col.min(line_len);
        let mut byte_offset = 0;
        for (i, ch) in line.chars().enumerate() {
            if i >= col {
                break;
            }
            byte_offset += ch.len_utf8();
        }
        line_start + byte_offset
    }

    pub fn visible_lines(&self, height: usize) -> Vec<(usize, String)> {
        let mut result = Vec::new();
        let start = self.scroll_top;
        let end = (start + height).min(self.line_count());
        for i in start..end {
            if let Some(line) = self.get_line(i) {
                result.push((i, line));
            }
        }
        result
    }

    pub fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.cursor_line < self.scroll_top {
            self.scroll_top = self.cursor_line;
        }
        if self.cursor_line >= self.scroll_top + height {
            self.scroll_top = self.cursor_line - height + 1;
        }
    }

    pub fn content(&self) -> String {
        self.rope.to_string()
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "[untitled]".to_string())
    }

    fn current_line_len(&self) -> usize {
        self.line_len(self.cursor_line)
    }

    fn line_len(&self, line: usize) -> usize {
        if line >= self.rope.len_lines() {
            return 0;
        }
        let line_slice = self.rope.line(line);
        let s = line_slice.to_string();
        let trimmed = s.trim_end_matches('\n').trim_end_matches('\r');
        trimmed.chars().count()
    }

    fn clamp_cursor_col(&mut self) {
        let len = self.current_line_len();
        if self.cursor_col > len {
            self.cursor_col = len;
        }
    }

    /// Reload buffer content from a string, preserving cursor position where possible.
    pub fn reload(&mut self, content: &str) {
        let old_line = self.cursor_line;
        let old_col = self.cursor_col;
        self.rope = Rope::from_str(content);
        self.cursor_line = old_line.min(self.line_count().saturating_sub(1));
        self.cursor_col = old_col;
        self.clamp_cursor_col();
        self.dirty = false;
    }
}

fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust".to_string(),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "javascript".to_string(),
        Some("ts") | Some("tsx") => "typescript".to_string(),
        Some("py") | Some("pyi") => "python".to_string(),
        Some("go") => "go".to_string(),
        Some("json") => "json".to_string(),
        Some("toml") => "toml".to_string(),
        Some("yml") | Some("yaml") => "yaml".to_string(),
        Some("md") | Some("markdown") => "markdown".to_string(),
        Some("sh") | Some("bash") | Some("zsh") => "bash".to_string(),
        Some("css") => "css".to_string(),
        Some("html") | Some("htm") => "html".to_string(),
        _ => "text".to_string(),
    }
}
