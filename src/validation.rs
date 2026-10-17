//! Input validation.
//!
//! All user-controlled values are checked against strict allowlists of
//! characters *before* they are used anywhere else (URLs, cache keys, logs,
//! file names). Anything outside these rules is rejected, never "cleaned up".

/// GitHub user and organization names: ASCII letters, digits and single
/// hyphens, 1-39 characters, no leading or trailing hyphen.
pub fn is_valid_owner(name: &str) -> bool {
    (1..=39).contains(&name.len())
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}

/// GitHub repository names: ASCII letters, digits, `-`, `_` and `.`,
/// 1-100 characters. `.`/`..` and any `..` sequence are rejected.
pub fn is_valid_repo(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && name != "."
        && !name.contains("..")
}

/// Version identifiers (tag names). This is a strict subset of valid git
/// ref names that is also safe to interpolate into shell scripts: it must
/// start with a letter or digit, may contain `-`, `_`, `.` and `+`, must not
/// contain `..` and must not end with `.` or `.lock`.
pub fn is_valid_version(version: &str) -> bool {
    (1..=100).contains(&version.len())
        && version.as_bytes()[0].is_ascii_alphanumeric()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+'))
        && !version.contains("..")
        && !version.ends_with('.')
        && !version.ends_with(".lock")
}

/// Names of generated diff archives: `diff-` + 32 lowercase hex chars + `.zip`.
pub fn is_valid_zip_filename(name: &str) -> bool {
    name.len() == 41
        && name.starts_with("diff-")
        && name.ends_with(".zip")
        && name[5..37]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A git object id (40 hex chars).
pub fn is_sha1_hex(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Checks that a path from a git diff can be written into a ZIP archive and
/// extracted without escaping the target directory.
///
/// Only printable ASCII and spaces are accepted (no Unicode line separators
/// or bidi controls that could confuse manifest parsers or reviewers).
/// Returns `None` for absolute paths, empty, `.` or `..` components, `.git`
/// directories, backslashes and colons. The path is never rewritten: an
/// unsafe path makes the whole archive fail.
pub fn safe_archive_path(path: &str) -> Option<&str> {
    if path.is_empty() || path.len() > 1024 || path.starts_with('/') {
        return None;
    }
    if !path
        .bytes()
        .all(|b| (b' '..=b'~').contains(&b) && b != b'\\' && b != b':')
    {
        return None;
    }
    let all_components_safe = path.split('/').all(|component| {
        !component.is_empty()
            && component != "."
            && component != ".."
            && !component.eq_ignore_ascii_case(".git")
    });
    all_components_safe.then_some(path)
}

/// Whether `ancestor` is a parent directory of `path` (`a` of `a/b`).
pub fn is_ancestor_path(ancestor: &str, path: &str) -> bool {
    path.len() > ancestor.len()
        && path.starts_with(ancestor)
        && path.as_bytes()[ancestor.len()] == b'/'
}

/// Normalizes a version string read from repository content (a `VERSION`
/// file or `package.json`). Returns `None` if it doesn't look like a version.
pub fn clean_version_string(raw: &str) -> Option<String> {
    let candidate = raw.lines().next()?.trim();
    (is_valid_version(candidate) && candidate != "latest").then(|| candidate.to_string())
}

/// Truncates free text from repository content to `max_chars` and strips
/// control characters.
pub fn clean_text(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owners() {
        for valid in ["Ym0T", "microsoft", "user-name", "a", &"a".repeat(39)] {
            assert!(is_valid_owner(valid), "{valid}");
        }
        for invalid in [
            "",
            "-user",
            "user-",
            "us--er",
            "user_name",
            "user.name",
            "us/er",
            "..",
            "user%2F",
            "üser",
            &"a".repeat(40),
        ] {
            assert!(!is_valid_owner(invalid), "{invalid}");
        }
    }

    #[test]
    fn repos() {
        for valid in [
            ".github",
            "repo.name",
            "repo_name",
            "pterodactyl-nginx-egg",
            &"r".repeat(100),
        ] {
            assert!(is_valid_repo(valid), "{valid}");
        }
        for invalid in [
            "",
            ".",
            "..",
            "a..b",
            "../../user",
            "repo?x",
            "repo#x",
            "repo/x",
            "repo%2e",
            "repo name",
            &"r".repeat(101),
        ] {
            assert!(!is_valid_repo(invalid), "{invalid}");
        }
    }

    #[test]
    fn versions() {
        for valid in [
            "v1.0.0",
            "1.2.3-beta.1",
            "v1.0.0+build.5",
            "main",
            "release_2024",
            "0123abc",
        ] {
            assert!(is_valid_version(valid), "{valid}");
        }
        for invalid in [
            "",
            ".v1",
            "-v1",
            "v1.",
            "v1..2",
            "%2e%2e",
            "a/b",
            "v1:v2",
            "v1?x",
            "v1#x",
            "v1 v2",
            "HEAD~1",
            "v1^",
            "v1.lock",
            "v1;rm",
            "$(id)",
            "v1`x`",
            "v1|x",
            "v1&x",
            &"v".repeat(101),
        ] {
            assert!(!is_valid_version(invalid), "{invalid}");
        }
    }

    #[test]
    fn zip_filenames() {
        assert!(is_valid_zip_filename(
            "diff-0123456789abcdef0123456789abcdef.zip"
        ));
        for invalid in [
            "diff-0123456789ABCDEF0123456789abcdef.zip",
            "diff-0123456789abcdef0123456789abcde.zip",
            "diff-0123456789abcdef0123456789abcdef.zip.part",
            "../diff-0123456789abcdef0123456789abcdef.zip",
            "diff-Ym0T-repo-v1-v2-abcd1234.zip",
            "",
        ] {
            assert!(!is_valid_zip_filename(invalid), "{invalid}");
        }
    }

    #[test]
    fn archive_paths() {
        for valid in [
            "README.md",
            "src/main.rs",
            ".github/workflows/ci.yml",
            "a/b/c.d",
            "....txt",
        ] {
            assert_eq!(safe_archive_path(valid), Some(valid));
        }
        for invalid in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "....//x",
            "....\\/x",
            "..././x",
            "a//b",
            "a/./b",
            "a/",
            ".git/config",
            "sub/.GIT/hooks/pre-commit",
            "C:/x",
            "a\\b",
            "a\nb",
            "a\0b",
            "a\u{2028}sha256=evil",
            "a\u{202e}txt.exe",
            "ümlaut.txt",
        ] {
            assert_eq!(safe_archive_path(invalid), None, "{invalid:?}");
        }
        assert_eq!(safe_archive_path("docs/a file.md"), Some("docs/a file.md"));
    }

    #[test]
    fn ancestor_paths() {
        assert!(is_ancestor_path("conf", "conf/default.conf"));
        assert!(is_ancestor_path("a/b", "a/b/c/d"));
        assert!(!is_ancestor_path("conf", "conf"));
        assert!(!is_ancestor_path("conf", "config/x"));
        assert!(!is_ancestor_path("a/b/c", "a/b"));
    }

    #[test]
    fn version_strings_from_content() {
        assert_eq!(clean_version_string("1.2.3\n").as_deref(), Some("1.2.3"));
        assert_eq!(
            clean_version_string("  v2.0.0+build.5  ").as_deref(),
            Some("v2.0.0+build.5")
        );
        assert_eq!(clean_version_string("latest"), None);
        assert_eq!(clean_version_string("<script>"), None);
        assert_eq!(clean_version_string(""), None);
        assert_eq!(clean_version_string(&"1".repeat(101)), None);
    }

    #[test]
    fn text_cleanup() {
        assert_eq!(clean_text("a\u{0}b\nc", 10), "abc");
        assert_eq!(clean_text("abcdef", 3), "abc");
    }
}
