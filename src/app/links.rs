//! File references in terminal output. Parsing is pure; resolution happens only
//! on an explicit link activation, outside terminal locks and rendering.

use std::path::{Path, PathBuf};

pub(super) fn url_from_target(
    target: crate::ghostty::LinkTarget,
    cwd: impl FnOnce() -> Option<PathBuf>,
) -> Option<String> {
    let (text, clicked_byte) = match target {
        crate::ghostty::LinkTarget::Uri(uri) => return Some(uri),
        crate::ghostty::LinkTarget::Text { text, clicked_byte } => (text, clicked_byte),
    };
    if let Some(range) = super::actions::url_byte_range(&text, clicked_byte) {
        return Some(text[range].to_owned());
    }
    let range = super::actions::file_path_byte_range(&text, clicked_byte)?;
    let path = Path::new(without_location(&text[range]));
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        cwd()?.join(path)
    };
    let path = path.canonicalize().ok()?;
    let metadata = path.metadata().ok()?;
    if !metadata.is_file() && !metadata.is_dir() {
        return None;
    }
    file_url(&path)
}

/// Location suffixes identify a file even when its default app cannot jump to a line.
pub(super) fn without_location(text: &str) -> &str {
    let numeric = |value: &str| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
    if let Some((path, location)) = text.rsplit_once("#L") {
        if numeric(location)
            || location
                .split_once("-L")
                .is_some_and(|(start, end)| numeric(start) && numeric(end))
        {
            return path;
        }
    }
    let mut path = text;
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((prefix, location)) if numeric(location) => path = prefix,
            _ => break,
        }
    }
    path
}

/// Encode native absolute paths, including Windows disk prefixes, without
/// exposing URL metacharacters in directory or file names to the opener.
fn file_url(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let path = crate::platform::local_file_url_path(path)?;
    let mut url = String::from("file://");
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~') {
            url.push(char::from(byte));
        } else {
            const HEX: &[u8] = b"0123456789ABCDEF";
            url.push('%');
            url.push(char::from(HEX[usize::from(byte >> 4)]));
            url.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    safe_local_file_url(&url).map(str::to_owned)
}

/// Accept only absolute file URLs with an empty authority. Callers must also
/// check that the response came from the local endpoint, never an SSH host.
pub(crate) fn safe_local_file_url(url: &str) -> Option<&str> {
    let path = url.strip_prefix("file://")?;
    if !path.starts_with('/') || path.starts_with("//") {
        return None;
    }
    let mut decoded = Vec::with_capacity(path.len());
    let mut bytes = path.bytes();
    while let Some(byte) = bytes.next() {
        decoded.push(if byte == b'%' {
            let hi = char::from(bytes.next()?).to_digit(16)?;
            let lo = char::from(bytes.next()?).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else if byte.is_ascii_alphanumeric()
            || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.' | b'~')
        {
            byte
        } else {
            return None;
        });
    }
    let decoded = String::from_utf8(decoded).ok()?;
    (decoded.starts_with('/')
        && !decoded.starts_with("//")
        && !decoded.chars().any(|ch| ch.is_control() || ch == '\\'))
    .then_some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::actions::link_byte_range;

    #[test]
    fn file_links_recognize_paths_wrappers_and_locations() {
        for (text, needle, expected) in [
            ("see `src/main.rs`.", "main", "src/main.rs"),
            ("(./README.md:42:7),", "42", "./README.md:42:7"),
            (
                "[source](../src/main.rs#L12-L18)",
                "main",
                "../src/main.rs#L12-L18",
            ),
            ("src/路径.rs:3", "路", "src/路径.rs:3"),
            ("update .gitignore", "git", ".gitignore"),
            ("edit Cargo.toml", "Cargo", "Cargo.toml"),
            ("run Makefile", "Make", "Makefile"),
            (r"src\main.rs:8", "main", r"src\main.rs:8"),
            (
                "https://example.com/src/main.rs:3",
                "main",
                "https://example.com/src/main.rs:3",
            ),
        ] {
            let range = link_byte_range(text, text.find(needle).unwrap()).unwrap();
            assert_eq!(&text[range], expected, "{text}");
        }
        for (text, click) in [
            ("ordinary", 2),
            ("`src/main.rs`.", 0),
            ("(src/main.rs),", 12),
            ("", 0),
        ] {
            assert!(link_byte_range(text, click).is_none(), "{text}");
        }
        assert_eq!(without_location("src/main.rs:42:7"), "src/main.rs");
        assert_eq!(without_location("src/main.rs#L12-L18"), "src/main.rs");
        assert_eq!(without_location(r"C:\src\main.rs:42"), r"C:\src\main.rs");
    }

    #[test]
    fn file_links_resolve_against_pane_cwd_and_escape_url_characters() {
        let root = std::env::temp_dir().join(format!("herdr-file-links-{}", std::process::id()));
        let cwd = root.join("space # % 路");
        std::fs::create_dir_all(cwd.join("src")).unwrap();
        std::fs::write(cwd.join("src/main.rs"), "fn main() {}\n").unwrap();
        let resolve = |text: &str, cwd: Option<PathBuf>| {
            url_from_target(
                crate::ghostty::LinkTarget::Text {
                    text: text.to_owned(),
                    clicked_byte: 3,
                },
                || cwd,
            )
        };
        let url = resolve("./src/main.rs:42:7", Some(cwd.clone())).unwrap();
        assert!(
            url.ends_with("/space%20%23%20%25%20%E8%B7%AF/src/main.rs"),
            "{url}"
        );
        assert!(safe_local_file_url(&url).is_some());
        assert!(resolve("./src/main.rs", Some(root.clone())).is_none());
        assert!(resolve("./src/main.rs", None).is_none());
        assert!(resolve("./missing.rs", Some(cwd.clone())).is_none());
        assert!(resolve("../src/main.rs", Some(cwd.join("src"))).is_some());
        assert!(url_from_target(
            crate::ghostty::LinkTarget::Uri("https://example.com".into()),
            || panic!("OSC links do not inspect cwd")
        )
        .is_some());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_links_reject_nonlocal_and_malformed_urls() {
        for url in [
            "file://host/tmp/a",
            "file://%2Ftmp/a",
            "file:////host/share",
            "file:///%2Fhost/share",
            "file:///%5Chost/share",
            "file:///tmp/%00",
            "file:///tmp/%GG",
            "file:///tmp/%",
            "file:relative",
            "file:///tmp/a#frag",
            "javascript:alert(1)",
        ] {
            assert!(safe_local_file_url(url).is_none(), "{url}");
        }
        assert!(safe_local_file_url("file:///tmp/a%20%23%25.txt").is_some());
        assert!(safe_local_file_url("file:///C:/src/main.rs").is_some());
    }

    #[test]
    fn file_links_keep_wrapped_unicode_ranges_and_activation_in_agreement() {
        let mut terminal = crate::ghostty::Terminal::new(12, 4, 1024).unwrap();
        terminal.write("`src/路径/main.rs:42`".as_bytes());
        for (col, row) in [(6, 0), (3, 1)] {
            let regions = terminal
                .viewport_link_regions(col, row, link_byte_range)
                .unwrap();
            assert_eq!(
                regions
                    .iter()
                    .map(|r| (r.row, r.start_col, r.end_col))
                    .collect::<Vec<_>>(),
                vec![(0, 1, 11), (1, 0, 7)]
            );
            let crate::ghostty::LinkTarget::Text { text, clicked_byte } =
                terminal.viewport_link_target(col, row).unwrap().unwrap()
            else {
                panic!("plain path");
            };
            let range = link_byte_range(&text, clicked_byte).unwrap();
            assert_eq!(&text[range], "src/路径/main.rs:42");
        }
    }
}
