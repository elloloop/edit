use similar::{ChangeTag, TextDiff};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub hunks: Vec<Hunk>,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub tag: DiffTag,
    pub content: String,
    pub old_lineno: Option<usize>,
    pub new_lineno: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffTag {
    Add,
    Delete,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Untracked,
}

#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
    pub review_state: ReviewState,
}

impl std::fmt::Display for ChangedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let marker = match self.status {
            FileStatus::Modified => "M",
            FileStatus::Added => "A",
            FileStatus::Deleted => "D",
            FileStatus::Untracked => "?",
        };
        let review = match self.review_state {
            ReviewState::Unread => " ",
            ReviewState::Reviewed => "*",
            ReviewState::EditedAfterReview => "!",
        };
        write!(f, "[{}] {} {}", marker, review, self.path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Unread,
    Reviewed,
    EditedAfterReview,
}

impl FileDiff {
    pub fn compute(old: &str, new: &str, path: &str) -> Self {
        let text_diff = TextDiff::from_lines(old, new);
        let mut hunks = Vec::new();
        let mut total_additions = 0usize;
        let mut total_deletions = 0usize;

        for group in text_diff.grouped_ops(3) {
            let mut hunk_lines = Vec::new();
            let first_op = &group[0];
            let last_op = group.last().unwrap();
            let old_start = first_op.old_range().start;
            let old_end = last_op.old_range().end;
            let new_start = first_op.new_range().start;
            let new_end = last_op.new_range().end;

            let mut old_lineno = old_start + 1;
            let mut new_lineno = new_start + 1;

            for op in &group {
                for change in text_diff.iter_changes(op) {
                    let content = change.to_string_lossy().trim_end_matches('\n').to_string();
                    match change.tag() {
                        ChangeTag::Equal => {
                            hunk_lines.push(DiffLine {
                                tag: DiffTag::Context,
                                content,
                                old_lineno: Some(old_lineno),
                                new_lineno: Some(new_lineno),
                            });
                            old_lineno += 1;
                            new_lineno += 1;
                        }
                        ChangeTag::Delete => {
                            hunk_lines.push(DiffLine {
                                tag: DiffTag::Delete,
                                content,
                                old_lineno: Some(old_lineno),
                                new_lineno: None,
                            });
                            old_lineno += 1;
                            total_deletions += 1;
                        }
                        ChangeTag::Insert => {
                            hunk_lines.push(DiffLine {
                                tag: DiffTag::Add,
                                content,
                                old_lineno: None,
                                new_lineno: Some(new_lineno),
                            });
                            new_lineno += 1;
                            total_additions += 1;
                        }
                    }
                }
            }

            hunks.push(Hunk {
                old_start: old_start + 1,
                old_count: old_end - old_start,
                new_start: new_start + 1,
                new_count: new_end - new_start,
                lines: hunk_lines,
            });
        }

        FileDiff {
            path: path.to_string(),
            hunks,
            additions: total_additions,
            deletions: total_deletions,
        }
    }
}

pub fn changed_files(repo_root: &Path) -> anyhow::Result<Vec<ChangedFile>> {
    let output = Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .arg("-uall")
        .current_dir(repo_root)
        .output()?;

    if !output.status.success() {
        anyhow::bail!("git status failed");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();

    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let status_chars: Vec<char> = line.chars().collect();
        let xy = (status_chars[0], status_chars[1]);
        let path = line[3..].to_string();

        let status = match xy {
            ('?', '?') => FileStatus::Untracked,
            ('A', _) | (_, 'A') => FileStatus::Added,
            ('D', _) | (_, 'D') => FileStatus::Deleted,
            _ => FileStatus::Modified,
        };

        files.push(ChangedFile {
            path,
            status,
            review_state: ReviewState::Unread,
        });
    }

    Ok(files)
}

/// Get the original (HEAD) content of a file for diffing
pub fn git_show_head(repo_root: &Path, file_path: &str) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!("HEAD:{}", file_path))
        .current_dir(repo_root)
        .output()?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Ok(String::new())
    }
}
