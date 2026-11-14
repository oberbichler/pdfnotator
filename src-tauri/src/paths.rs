//! Path handling for documents passed in by agents and the CLI.

use std::path::{Path, PathBuf};

/// Kind of document the viewer can open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// A Typst source file — compiled by pdfnotator itself.
    Typst,
    /// A ready-made PDF — only watched for changes.
    Pdf,
}

/// Normalise a user/agent supplied path: trims whitespace, strips `file://`,
/// expands a leading `~` and requires the result to be absolute.
pub fn normalize(raw: &str) -> Result<PathBuf, String> {
    let raw = raw.trim();
    let raw = raw.strip_prefix("file://").unwrap_or(raw);
    if raw.is_empty() {
        return Err("path is empty".into());
    }
    let mut path = PathBuf::from(raw);
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = home_dir()
    {
        path = home.join(rest);
    }
    if !path.is_absolute() {
        return Err(format!("path must be absolute: {raw}"));
    }
    Ok(path)
}

/// Decide how to treat a document based on its extension.
pub fn kind_of(path: &Path) -> Result<DocKind, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("typ") => Ok(DocKind::Typst),
        Some("pdf") => Ok(DocKind::Pdf),
        _ => Err(format!(
            "unsupported file type (expected .typ or .pdf): {}",
            path.display()
        )),
    }
}

/// Resolve CLI arguments relative to the current directory.
pub fn absolutize(raw: &str) -> String {
    std::fs::canonicalize(raw).map_or_else(|_| raw.to_owned(), |p| p.display().to_string())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_relative_and_empty_paths() {
        assert!(normalize("doc.pdf").unwrap_err().contains("absolute"));
        assert!(normalize("   ").unwrap_err().contains("empty"));
    }

    #[cfg(unix)]
    #[test]
    fn strips_file_scheme_and_whitespace() {
        assert_eq!(
            normalize("  file:///tmp/a.pdf \n").unwrap(),
            PathBuf::from("/tmp/a.pdf")
        );
    }

    #[cfg(unix)]
    #[test]
    fn expands_home() {
        let home = home_dir().unwrap();
        assert_eq!(normalize("~/x/a.typ").unwrap(), home.join("x/a.typ"));
    }

    #[test]
    fn detects_kind_case_insensitively() {
        assert_eq!(kind_of(Path::new("/a/b.TYP")).unwrap(), DocKind::Typst);
        assert_eq!(kind_of(Path::new("/a/b.pdf")).unwrap(), DocKind::Pdf);
        assert!(kind_of(Path::new("/a/b.docx")).is_err());
        assert!(kind_of(Path::new("/a/b")).is_err());
    }
}
