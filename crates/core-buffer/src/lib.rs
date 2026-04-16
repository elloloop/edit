use ropey::Rope;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
    FileStart,
    FileEnd,
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
    pub is_binary: bool,
    pub language: String,
    pub cursor_line: usize,
    pub cursor_col: usize,
    pub scroll_top: usize,
    pub scroll_left: usize,
    pub selection: Option<Selection>,
    history: Vec<Rope>,
    undo_pos: usize,
}

impl Buffer {
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let bytes = fs::read(path)?;
        let is_binary = bytes.contains(&0);
        let content = if is_binary {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            String::from_utf8(bytes)?
        };
        let language = detect_language(path);
        Ok(Self {
            rope: Rope::from_str(&content),
            path: Some(path.to_path_buf()),
            dirty: false,
            is_binary,
            language,
            cursor_line: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_left: 0,
            selection: None,
            history: vec![Rope::from_str(&content)],
            undo_pos: 0,
        })
    }

    pub fn from_string(content: &str) -> Self {
        Self {
            rope: Rope::from_str(content),
            path: None,
            dirty: false,
            is_binary: false,
            language: String::from("text"),
            cursor_line: 0,
            cursor_col: 0,
            scroll_top: 0,
            scroll_left: 0,
            selection: None,
            history: vec![Rope::from_str(content)],
            undo_pos: 0,
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
        self.prepare_history();
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
        self.save_snapshot();
    }

    pub fn delete_char(&mut self) {
        let line_len = self.current_line_len();
        if self.cursor_col < line_len {
            self.prepare_history();
            let idx = self.cursor_byte_offset();
            let char_idx = self.rope.byte_to_char(idx);
            if char_idx < self.rope.len_chars() {
                self.rope.remove(char_idx..char_idx + 1);
                self.dirty = true;
                self.save_snapshot();
            }
        } else if self.cursor_line + 1 < self.line_count() {
            // Join with next line
            self.prepare_history();
            let idx = self.cursor_byte_offset();
            let char_idx = self.rope.byte_to_char(idx);
            if char_idx < self.rope.len_chars() {
                self.rope.remove(char_idx..char_idx + 1);
                self.dirty = true;
                self.save_snapshot();
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
        match direction {
            Direction::PageUp => {
                self.cursor_line = self.cursor_line.saturating_sub(count);
                self.clamp_cursor_col();
                return;
            }
            Direction::PageDown => {
                self.cursor_line =
                    (self.cursor_line + count).min(self.line_count().saturating_sub(1));
                self.clamp_cursor_col();
                return;
            }
            Direction::FileStart => {
                self.cursor_line = 0;
                self.cursor_col = 0;
                return;
            }
            Direction::FileEnd => {
                self.cursor_line = self.line_count().saturating_sub(1);
                self.cursor_col = self.current_line_len();
                return;
            }
            _ => {}
        }

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
                Direction::WordLeft => self.move_word_left(),
                Direction::WordRight => self.move_word_right(),
                Direction::Home => {
                    self.cursor_col = 0;
                }
                Direction::End => {
                    self.cursor_col = self.current_line_len();
                }
                Direction::FileStart
                | Direction::FileEnd
                | Direction::PageUp
                | Direction::PageDown => {}
            }
        }
    }

    pub fn go_to_line(&mut self, line: usize) {
        let target = line.min(self.line_count().saturating_sub(1));
        self.cursor_line = target;
        self.clamp_cursor_col();
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn select_range(
        &mut self,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) {
        if self.line_count() == 0 {
            self.selection = None;
            self.cursor_line = 0;
            self.cursor_col = 0;
            return;
        }

        let start_line = start_line.min(self.line_count().saturating_sub(1));
        let end_line = end_line.min(self.line_count().saturating_sub(1));
        let start_col = start_col.min(self.line_len(start_line));
        let end_col = end_col.min(self.line_len(end_line));

        self.selection = Some(Selection {
            start_line,
            start_col,
            end_line,
            end_col,
        });
        self.cursor_line = end_line;
        self.cursor_col = end_col;
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
        let margin = 3usize.min(height.saturating_sub(1));
        let top_guard = self.scroll_top.saturating_add(margin);
        let bottom_guard = self
            .scroll_top
            .saturating_add(height.saturating_sub(margin + 1));

        if self.cursor_line < top_guard {
            self.scroll_top = self.cursor_line.saturating_sub(margin);
        } else if self.cursor_line > bottom_guard {
            self.scroll_top = self
                .cursor_line
                .saturating_add(margin + 1)
                .saturating_sub(height);
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
        self.history.clear();
        self.history.push(self.rope.clone());
        self.undo_pos = 0;
    }

    pub fn replace_content(&mut self, content: &str) {
        self.prepare_history();
        self.rope = Rope::from_str(content);
        self.fix_cursor_after_history_change();
        self.dirty = true;
        self.save_snapshot();
    }

    pub fn undo(&mut self) -> bool {
        if self.undo_pos == 0 {
            return false;
        }
        self.undo_pos -= 1;
        self.rope = self.history[self.undo_pos].clone();
        self.fix_cursor_after_history_change();
        self.dirty = self.undo_pos != 0;
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.undo_pos + 1 >= self.history.len() {
            return false;
        }
        self.undo_pos += 1;
        self.rope = self.history[self.undo_pos].clone();
        self.fix_cursor_after_history_change();
        self.dirty = self.undo_pos != 0;
        true
    }

    fn move_word_left(&mut self) {
        if self.cursor_line == 0 && self.cursor_col == 0 {
            return;
        }

        let chars: Vec<char> = self.current_line_chars();
        if self.cursor_col == 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.line_len(self.cursor_line);
            self.move_word_left();
            return;
        }

        let mut idx = self.cursor_col.min(chars.len());
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && is_word_char(chars[idx - 1]) {
            idx -= 1;
        }
        if idx == self.cursor_col && idx > 0 {
            idx -= 1;
        }
        self.cursor_col = idx;
    }

    fn move_word_right(&mut self) {
        let chars: Vec<char> = self.current_line_chars();
        if self.cursor_col >= chars.len() {
            if self.cursor_line + 1 < self.line_count() {
                self.cursor_line += 1;
                self.cursor_col = 0;
                self.move_word_right();
            }
            return;
        }

        let mut idx = self.cursor_col;
        while idx < chars.len() && chars[idx].is_whitespace() {
            idx += 1;
        }
        while idx < chars.len() && is_word_char(chars[idx]) {
            idx += 1;
        }
        if idx == self.cursor_col && idx < chars.len() {
            idx += 1;
        }
        self.cursor_col = idx;
    }

    fn current_line_chars(&self) -> Vec<char> {
        self.get_line(self.cursor_line)
            .unwrap_or_default()
            .chars()
            .collect()
    }

    fn prepare_history(&mut self) {
        if self.undo_pos + 1 < self.history.len() {
            self.history.truncate(self.undo_pos + 1);
        }
        if self.history.is_empty() {
            self.history.push(self.rope.clone());
            self.undo_pos = 0;
        }
    }

    fn save_snapshot(&mut self) {
        self.history.push(self.rope.clone());
        while self.history.len() > 100 {
            self.history.remove(0);
        }
        self.undo_pos = self.history.len().saturating_sub(1);
    }

    fn fix_cursor_after_history_change(&mut self) {
        self.cursor_line = self.cursor_line.min(self.line_count().saturating_sub(1));
        self.clamp_cursor_col();
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

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::{Buffer, Direction};

    #[test]
    fn word_movement_respects_identifiers() {
        let mut buffer = Buffer::from_string("alpha beta_gamma");
        buffer.cursor_col = buffer.get_line(0).unwrap().chars().count();
        buffer.move_cursor(Direction::WordLeft, 1);
        assert_eq!(buffer.cursor_col, 6);
        buffer.move_cursor(Direction::WordLeft, 1);
        assert_eq!(buffer.cursor_col, 0);
        buffer.move_cursor(Direction::WordRight, 1);
        assert_eq!(buffer.cursor_col, 5);
        buffer.move_cursor(Direction::WordRight, 1);
        assert_eq!(buffer.cursor_col, 16);
    }

    #[test]
    fn undo_and_redo_restore_text() {
        let mut buffer = Buffer::from_string("abc");
        buffer.cursor_col = 3;
        buffer.insert_char('d');
        buffer.new_line();
        buffer.insert_char('x');
        assert_eq!(buffer.content(), "abcd\nx");

        assert!(buffer.undo());
        assert_eq!(buffer.content(), "abcd\n");
        assert!(buffer.undo());
        assert_eq!(buffer.content(), "abcd");
        assert!(buffer.undo());
        assert_eq!(buffer.content(), "abc");

        assert!(buffer.redo());
        assert_eq!(buffer.content(), "abcd");
        assert!(buffer.redo());
        assert_eq!(buffer.content(), "abcd\n");
        assert!(buffer.redo());
        assert_eq!(buffer.content(), "abcd\nx");
    }

    #[test]
    fn page_movement_uses_requested_size() {
        let text = (0..200)
            .map(|n| format!("line {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut buffer = Buffer::from_string(&text);
        buffer.move_cursor(Direction::PageDown, 17);
        assert_eq!(buffer.cursor_line, 17);
        buffer.move_cursor(Direction::PageUp, 9);
        assert_eq!(buffer.cursor_line, 8);
    }

    #[test]
    fn select_range_clamps_and_updates_cursor() {
        let mut buffer = Buffer::from_string("alpha\nbeta\n");
        buffer.select_range(0, 2, 8, 99);

        let selection = buffer.selection.as_ref().expect("selection should be set");
        assert_eq!(selection.start_line, 0);
        assert_eq!(selection.start_col, 2);
        assert_eq!(selection.end_line, 2);
        assert_eq!(selection.end_col, 0);
        assert_eq!(buffer.cursor_line, 2);
        assert_eq!(buffer.cursor_col, 0);

        buffer.clear_selection();
        assert!(buffer.selection.is_none());
    }
}
