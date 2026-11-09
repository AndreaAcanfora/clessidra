//! Which paths are worth keeping. Heavy, regenerable directories (dependencies, build output,
//! version control internals) are skipped by default; a `.clessidraignore` file at the root of a
//! watched folder adds more patterns in `.gitignore` syntax. `.gitignore` itself is deliberately
//! not honoured: the files it hides (`.env`, local notes, scratch data) are exactly the ones that
//! have no other copy anywhere.

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Patterns ignored in every watched folder.
pub const DEFAULT_IGNORES: &[&str] = &[
    ".git/",
    ".hg/",
    ".svn/",
    ".jj/",
    "node_modules/",
    "target/",
    ".next/",
    ".nuxt/",
    ".turbo/",
    ".venv/",
    "venv/",
    "__pycache__/",
    ".pytest_cache/",
    ".mypy_cache/",
    ".gradle/",
    ".cache/",
    ".DS_Store",
    "Thumbs.db",
    "*.swp",
    "*.swx",
    "*~",
    ".#*",
    "*.tmp",
    ".~lock.*",
    "*.crdownload",
    "*.part",
    "*.clessidra-tmp",
];

pub const IGNORE_FILE: &str = ".clessidraignore";

pub struct Filter {
    root: PathBuf,
    rules: Gitignore,
    /// The store's own directory, when it happens to live inside the watched folder.
    exclude: Option<PathBuf>,
}

impl Filter {
    pub fn new(root: &Path, exclude: Option<&Path>) -> Self {
        let mut b = GitignoreBuilder::new(root);
        for p in DEFAULT_IGNORES {
            let _ = b.add_line(None, p);
        }
        let custom = root.join(IGNORE_FILE);
        if custom.is_file()
            && let Some(e) = b.add(&custom)
        {
            log::warn!("{}: {e}", custom.display());
        }
        let rules = b.build().unwrap_or_else(|_| Gitignore::empty());
        Filter {
            root: root.to_path_buf(),
            rules,
            exclude: exclude.map(Path::to_path_buf),
        }
    }

    /// Whether an absolute path inside the root should be left alone.
    pub fn ignores(&self, abs: &Path, is_dir: bool) -> bool {
        if let Some(ex) = &self.exclude
            && abs.starts_with(ex)
        {
            return true;
        }
        if abs == self.root {
            return false;
        }
        match abs.strip_prefix(&self.root) {
            Ok(_) => self.rules.matched_path_or_any_parents(abs, is_dir).is_ignore(),
            Err(_) => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_custom_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(IGNORE_FILE), "*.log\nbig/\n").unwrap();
        let f = Filter::new(root, None);
        assert!(f.ignores(&root.join("node_modules/x/index.js"), false));
        assert!(f.ignores(&root.join("a/.git/HEAD"), false));
        assert!(f.ignores(&root.join("server.log"), false));
        assert!(f.ignores(&root.join("big/blob.bin"), false));
        assert!(!f.ignores(&root.join(".env"), false));
        assert!(!f.ignores(&root.join("src/main.rs"), false));
        assert!(!f.ignores(root, true));
    }
}
