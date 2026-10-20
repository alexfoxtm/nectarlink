// SPDX-License-Identifier: GPL-3.0-or-later
//! Accept/reject rules for file lists moved through the pasteboard. Pure
//! (no AppKit, no filesystem), so they are unit-testable without touching
//! the user's pasteboard.

use std::path::{Component, Path, PathBuf};

/// Most files read from or written to the pasteboard at once. A longer
/// list is refused whole, never silently truncated.
pub(super) const MAX_FILES: usize = 256;

/// Longest accepted path in bytes (`PATH_MAX` minus its NUL).
pub(super) const MAX_PATH_BYTES: usize = 1023;

const FILE_SCHEME: &str = "file://";

/// Longest accepted file URL: a fully percent-encoded maximal path
/// under the `localhost` host.
const MAX_URL_BYTES: usize = FILE_SCHEME.len() + "localhost".len() + 3 * MAX_PATH_BYTES;

/// A local absolute path within bounds, with no `..` and no NUL.
fn checked_path(path: &str) -> Option<PathBuf> {
    let ok = path.starts_with('/')
        && path.len() <= MAX_PATH_BYTES
        && !path.contains('\0')
        && !Path::new(path).components().any(|c| c == Component::ParentDir);
    ok.then(|| PathBuf::from(path))
}

/// Strict `%XX` decoding; raw spaces and ASCII controls are malformed.
fn percent_decode(encoded: &str) -> Option<Vec<u8>> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = bytes.get(i + 1..i + 3)?;
                if !hex.iter().all(u8::is_ascii_hexdigit) {
                    return None;
                }
                out.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
                i += 3;
            }
            b if b <= b' ' || b == 0x7f => return None,
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    Some(out)
}

/// The local path a `file://` URL names. Remote hosts, queries,
/// fragments, malformed escapes and non-UTF-8 paths are refused.
fn path_from_file_url(url: &str) -> Option<PathBuf> {
    if url.len() > MAX_URL_BYTES || !url.get(..FILE_SCHEME.len())?.eq_ignore_ascii_case(FILE_SCHEME) {
        return None;
    }
    let rest = &url[FILE_SCHEME.len()..];
    let (host, encoded) = rest.split_at(rest.find('/')?);
    if !(host.is_empty() || host.eq_ignore_ascii_case("localhost")) || encoded.contains(['?', '#']) {
        return None;
    }
    checked_path(&String::from_utf8(percent_decode(encoded)?).ok()?)
}

/// Paths for the file URLs read off the pasteboard; `None` when the list
/// is empty, too long, or any entry is not a well-formed local file URL.
pub(super) fn paths_from_file_urls(urls: &[String]) -> Option<Vec<PathBuf>> {
    if urls.is_empty() || urls.len() > MAX_FILES {
        return None;
    }
    urls.iter().map(|url| path_from_file_url(url)).collect()
}

/// Paths to place on the pasteboard, as the UTF-8 strings `NSURL` takes.
pub(super) fn validate_write_paths(paths: &[PathBuf]) -> Result<Vec<String>, String> {
    if paths.is_empty() || paths.len() > MAX_FILES {
        return Err(format!("between 1 and {MAX_FILES} files can be copied at once"));
    }
    paths
        .iter()
        .map(|path| {
            path.to_str()
                .filter(|s| checked_path(s).is_some())
                .map(str::to_owned)
                .ok_or_else(|| "not a local absolute UTF-8 file path".to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

    #[test]
    fn file_urls_and_paths_keep_unicode_and_spaces() {
        let urls = [
            "file:///Users/alice/Documents/Q3%20report.pdf",
            "file://localhost/Users/alice/Photos/Summer%202026/beach_%F0%9F%90%9D.jpg",
            "FILE:///Users/alice/%D0%9F%D1%80%D0%B8%D0%B2%D0%B5%D1%82/",
        ]
        .map(String::from);
        let expected = vec![
            PathBuf::from("/Users/alice/Documents/Q3 report.pdf"),
            PathBuf::from("/Users/alice/Photos/Summer 2026/beach_🐝.jpg"),
            PathBuf::from("/Users/alice/Привет"),
        ];
        assert_eq!(paths_from_file_urls(&urls), Some(expected.clone()));
        let written = validate_write_paths(&expected).expect("valid paths");
        assert_eq!(written[1], "/Users/alice/Photos/Summer 2026/beach_🐝.jpg");
    }

    #[test]
    fn malformed_foreign_or_unbounded_file_lists_are_refused() {
        for url in [
            "",
            "https://example.com/a.txt",
            "file://server/share/a.txt",
            "file:relative/a.txt",
            "file://localhost",
            "file:///tmp/a%00b",
            "file:///tmp/a%2",
            "file:///tmp/a%zz",
            "file:///tmp/a%+f",
            "file:///tmp/%FF",
            "file:///tmp/a b",
            "file:///tmp/a?x=1",
            "file:///tmp/a#frag",
            "file:///tmp/../etc/passwd",
        ] {
            assert_eq!(paths_from_file_urls(&[url.to_owned()]), None, "{url:?}");
        }
        assert_eq!(paths_from_file_urls(&[]), None);
        assert_eq!(paths_from_file_urls(&["file:///tmp/ok".to_owned(), "https://x/y".to_owned()]), None);
        let too_many: Vec<String> = (0..=MAX_FILES).map(|_| "file:///tmp/a".to_owned()).collect();
        assert_eq!(paths_from_file_urls(&too_many), None);

        let long = format!("/{}", "a".repeat(MAX_PATH_BYTES));
        for path in ["relative/a.txt", "/tmp/a\0b", "/tmp/../etc", long.as_str()] {
            assert!(validate_write_paths(&[PathBuf::from(path)]).is_err(), "{path:?}");
        }
        assert!(validate_write_paths(&[PathBuf::from(OsStr::from_bytes(b"/tmp/\xff"))]).is_err());
        assert!(validate_write_paths(&[]).is_err());
        let too_many: Vec<PathBuf> = (0..=MAX_FILES).map(|_| PathBuf::from("/tmp/a")).collect();
        assert!(validate_write_paths(&too_many).is_err());
    }
}
