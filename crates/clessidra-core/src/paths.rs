//! Paths inside a watched folder are stored relative to it, always with `/` separators.

use std::path::{Component, Path, PathBuf};

/// `abs` relative to `root` as a `/`-separated string, or `None` when it is not inside `root`.
/// The root itself maps to the empty string.
pub fn relative(root: &Path, abs: &Path) -> Option<String> {
    let rest = abs.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rest.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

/// The absolute path of a stored relative path.
pub fn absolute(root: &Path, rel: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

/// Whether `rel` is `prefix` itself or lies under it. The empty prefix contains everything.
pub fn within(rel: &str, prefix: &str) -> bool {
    prefix.is_empty() || rel == prefix || (rel.starts_with(prefix) && rel.as_bytes().get(prefix.len()) == Some(&b'/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_and_back() {
        let root = Path::new("/a/b");
        assert_eq!(relative(root, Path::new("/a/b/c/d.txt")).as_deref(), Some("c/d.txt"));
        assert_eq!(relative(root, Path::new("/a/b")).as_deref(), Some(""));
        assert_eq!(relative(root, Path::new("/a/bc/d")), None);
        assert_eq!(absolute(root, "c/d.txt"), PathBuf::from("/a/b/c/d.txt"));
    }

    #[test]
    fn within_respects_segment_boundaries() {
        assert!(within("src/main.rs", "src"));
        assert!(within("src", "src"));
        assert!(!within("srcs/x", "src"));
        assert!(within("anything", ""));
    }
}
