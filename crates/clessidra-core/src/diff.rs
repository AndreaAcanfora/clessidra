//! Line diffs between two versions of a file, shaped for display.

use serde::Serialize;
use similar::{ChangeTag, TextDiff};

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
    /// A marker standing for unchanged lines that were left out.
    Gap,
}

#[derive(Serialize, Clone, Debug)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Diff {
    /// False when either side is not UTF-8 text; `lines` is then empty.
    pub text: bool,
    pub added: usize,
    pub removed: usize,
    pub lines: Vec<DiffLine>,
}

/// Is this content text we can show? Valid UTF-8 without NUL bytes.
pub fn as_text(bytes: &[u8]) -> Option<&str> {
    if bytes.contains(&0) {
        return None;
    }
    std::str::from_utf8(bytes).ok()
}

pub fn diff(old: &[u8], new: &[u8], context: usize) -> Diff {
    let (Some(a), Some(b)) = (as_text(old), as_text(new)) else {
        return Diff {
            text: false,
            added: 0,
            removed: 0,
            lines: Vec::new(),
        };
    };
    let td = TextDiff::from_lines(a, b);
    let mut lines = Vec::new();
    let (mut added, mut removed) = (0, 0);
    for (i, group) in td.grouped_ops(context).iter().enumerate() {
        if i > 0 {
            lines.push(DiffLine {
                kind: DiffLineKind::Gap,
                text: String::new(),
                old_line: None,
                new_line: None,
            });
        }
        for op in group {
            for change in td.iter_changes(op) {
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffLineKind::Context,
                    ChangeTag::Insert => {
                        added += 1;
                        DiffLineKind::Added
                    }
                    ChangeTag::Delete => {
                        removed += 1;
                        DiffLineKind::Removed
                    }
                };
                let text = change.value().trim_end_matches(['\n', '\r']).to_string();
                lines.push(DiffLine {
                    kind,
                    text,
                    old_line: change.old_index().map(|n| n + 1),
                    new_line: change.new_index().map(|n| n + 1),
                });
            }
        }
    }
    Diff {
        text: true,
        added,
        removed,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_numbers() {
        let d = diff(b"a\nb\nc\n", b"a\nB\nc\nd\n", 3);
        assert!(d.text);
        assert_eq!((d.added, d.removed), (2, 1));
        let removed = d.lines.iter().find(|l| l.kind == DiffLineKind::Removed).unwrap();
        assert_eq!((removed.text.as_str(), removed.old_line), ("b", Some(2)));
    }

    #[test]
    fn binary_is_not_diffed() {
        assert!(!diff(b"\0\x01", b"x", 3).text);
    }
}
