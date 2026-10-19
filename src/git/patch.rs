//! Byte-exact parsing and re-assembly of `git diff` output.
//!
//! The parser never re-serialises anything: every hunk and header is a verbatim
//! slice of git's own output, so what gets fed to `git apply` is exactly what
//! git produced. The only text that is ever rewritten is the line numbers in a
//! hunk header, when hunks of one file are split across several commits.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_count: u32,
    pub new_start: u32,
    pub new_count: u32,
    pub added: u32,
    pub removed: u32,
    /// From the `@@` line through the last body line (including `\ No newline` markers).
    pub text: Vec<u8>,
}

impl Hunk {
    pub fn delta(&self) -> i64 {
        i64::from(self.new_count) - i64::from(self.old_count)
    }

    /// The hunk body without the `@@` header line.
    pub fn body(&self) -> &[u8] {
        match self.text.iter().position(|b| *b == b'\n') {
            Some(i) => &self.text[i + 1..],
            None => &[],
        }
    }

    /// Text after the second `@@` of the header (the function context), including the newline.
    fn header_suffix(&self) -> &[u8] {
        let line_end = self
            .text
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(self.text.len());
        let line = &self.text[..line_end];
        // Find the closing "@@" after the leading "@@ ".
        if let Some(pos) = find(&line[2..], b"@@") {
            &line[2 + pos + 2..]
        } else {
            b"\n"
        }
    }

    /// Renders the header with new positions, keeping the original function context.
    pub fn header_with(&self, old_start: u32, new_start: u32) -> Vec<u8> {
        let mut out = format!(
            "@@ -{},{} +{},{} @@",
            old_start, self.old_count, new_start, self.new_count
        )
        .into_bytes();
        out.extend_from_slice(self.header_suffix());
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Modified,
    Added,
    Deleted,
    /// File replaced by a symlink or vice versa; git emits two sections for one path.
    TypeChange,
    /// Mode bits changed, content did not.
    Mode,
    /// A submodule pointer (gitlink) was added, removed or moved.
    Submodule,
}

impl fmt::Display for FileKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            FileKind::Modified => "modified",
            FileKind::Added => "new file",
            FileKind::Deleted => "deleted",
            FileKind::TypeChange => "type change",
            FileKind::Mode => "mode change",
            FileKind::Submodule => "submodule",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    /// The path as git prints it (new path, or the old path for deletions).
    pub path: String,
    pub kind: FileKind,
    pub binary: bool,
    /// `(old, new)` when the header carries `old mode`/`new mode` lines.
    pub mode_change: Option<(String, String)>,
    /// `diff --git` line through the `+++` line (or the whole section for hunk-less files).
    pub header: Vec<u8>,
    pub hunks: Vec<Hunk>,
    /// Every section for this path, verbatim.
    pub raw: Vec<u8>,
}

impl FilePatch {
    /// True when the whole file must be committed as one unit.
    pub fn atomic(&self) -> bool {
        self.binary || self.kind != FileKind::Modified || self.hunks.len() <= 1
    }

    pub fn added(&self) -> u32 {
        self.hunks.iter().map(|h| h.added).sum()
    }

    pub fn removed(&self) -> u32 {
        self.hunks.iter().map(|h| h.removed).sum()
    }

    /// Old and new submodule commit ids, when this is a submodule change.
    pub fn submodule_commits(&self) -> Option<(Option<String>, Option<String>)> {
        if self.kind != FileKind::Submodule {
            return None;
        }
        let mut old = None;
        let mut new = None;
        for hunk in &self.hunks {
            for line in hunk.body().split(|b| *b == b'\n') {
                let line = String::from_utf8_lossy(line);
                if let Some(rest) = line.strip_prefix("-Subproject commit ") {
                    old = Some(rest.trim().to_string());
                } else if let Some(rest) = line.strip_prefix("+Subproject commit ") {
                    new = Some(rest.trim().to_string());
                }
            }
        }
        Some((old, new))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("malformed hunk header: {0}")]
    HunkHeader(String),
    #[error("could not determine the file path of a diff section: {0}")]
    Path(String),
}

const SECTION_START: &[u8] = b"diff --git ";

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Splits data into lines, keeping line terminators.
fn lines(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in data.iter().enumerate() {
        if *b == b'\n' {
            out.push(&data[start..=i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}

/// Parses one `git diff` (or `git diff-index -p`) output into per-file patches.
pub fn parse_patch(data: &[u8]) -> Result<Vec<FilePatch>, PatchError> {
    let all = lines(data);
    // Section boundaries.
    let mut starts: Vec<usize> = all
        .iter()
        .enumerate()
        .filter(|(_, l)| l.starts_with(SECTION_START))
        .map(|(i, _)| i)
        .collect();
    let mut files: Vec<FilePatch> = Vec::new();
    starts.push(all.len());
    for pair in starts.windows(2) {
        let section = &all[pair[0]..pair[1]];
        let file = parse_section(section)?;
        // git emits two sections for a type change (delete + add of the same path).
        if let Some(prev) = files.last_mut() {
            if prev.path == file.path && !prev.binary && !file.binary {
                prev.kind = FileKind::TypeChange;
                prev.raw.extend_from_slice(&file.raw);
                prev.hunks.clear();
                prev.mode_change = None;
                continue;
            }
        }
        files.push(file);
    }
    Ok(files)
}

fn parse_section(section: &[&[u8]]) -> Result<FilePatch, PatchError> {
    let raw: Vec<u8> = section.concat();
    let first = section[0];

    // Header ends at the first hunk, binary marker, or end of section.
    let mut header_end = section.len();
    let mut binary = false;
    for (i, line) in section.iter().enumerate().skip(1) {
        if line.starts_with(b"@@ ") {
            header_end = i;
            break;
        }
        if line.starts_with(b"GIT binary patch") || line.starts_with(b"Binary files ") {
            binary = true;
            header_end = i;
            break;
        }
    }
    let header_lines = &section[..header_end];
    let header: Vec<u8> = header_lines.concat();

    let mut is_new = false;
    let mut is_deleted = false;
    let mut old_mode = None;
    let mut new_mode = None;
    let mut gitlink = false;
    let mut minus_path: Option<String> = None;
    let mut plus_path: Option<String> = None;
    for line in header_lines.iter().skip(1) {
        let text = String::from_utf8_lossy(line);
        let text = text.trim_end_matches(['\n', '\r']);
        if let Some(mode) = text.strip_prefix("new file mode ") {
            is_new = true;
            gitlink |= mode.trim() == "160000";
        } else if let Some(mode) = text.strip_prefix("deleted file mode ") {
            is_deleted = true;
            gitlink |= mode.trim() == "160000";
        } else if let Some(mode) = text.strip_prefix("old mode ") {
            old_mode = Some(mode.trim().to_string());
        } else if let Some(mode) = text.strip_prefix("new mode ") {
            new_mode = Some(mode.trim().to_string());
            gitlink |= mode.trim() == "160000";
        } else if let Some(rest) = text.strip_prefix("index ") {
            if rest.split_whitespace().nth(1) == Some("160000") {
                gitlink = true;
            }
        } else if let Some(rest) = text.strip_prefix("--- ") {
            minus_path = Some(strip_prefix_and_unquote(rest));
        } else if let Some(rest) = text.strip_prefix("+++ ") {
            plus_path = Some(strip_prefix_and_unquote(rest));
        }
    }

    let path = match (&plus_path, &minus_path) {
        (Some(p), _) if p != "/dev/null" => p.clone(),
        (_, Some(m)) if m != "/dev/null" => m.clone(),
        _ => path_from_diff_git_line(first)?,
    };

    // Hunks.
    let mut hunks = Vec::new();
    if !binary {
        let mut i = header_end;
        while i < section.len() {
            let line = section[i];
            if !line.starts_with(b"@@ ") {
                i += 1;
                continue;
            }
            let (old_start, old_count, new_start, new_count) = parse_hunk_header(line)?;
            let mut text: Vec<u8> = line.to_vec();
            let mut added = 0;
            let mut removed = 0;
            i += 1;
            while i < section.len() {
                let body = section[i];
                match body.first() {
                    Some(b' ') | Some(b'\\') => {}
                    Some(b'+') => added += 1,
                    Some(b'-') => removed += 1,
                    // An empty context line ("\n") can appear when git omits the leading space.
                    Some(b'\n') | Some(b'\r') => {}
                    _ => break,
                }
                text.extend_from_slice(body);
                i += 1;
            }
            hunks.push(Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                added,
                removed,
                text,
            });
        }
    }

    let mode_change = match (old_mode, new_mode) {
        (Some(o), Some(n)) => Some((o, n)),
        _ => None,
    };
    let kind = if gitlink {
        FileKind::Submodule
    } else if is_new {
        FileKind::Added
    } else if is_deleted {
        FileKind::Deleted
    } else if hunks.is_empty() && !binary && mode_change.is_some() {
        FileKind::Mode
    } else {
        FileKind::Modified
    };

    Ok(FilePatch {
        path,
        kind,
        binary,
        mode_change,
        header,
        hunks,
        raw,
    })
}

fn parse_hunk_header(line: &[u8]) -> Result<(u32, u32, u32, u32), PatchError> {
    let text = String::from_utf8_lossy(line);
    let err = || PatchError::HunkHeader(text.trim_end().to_string());
    let rest = text.strip_prefix("@@ ").ok_or_else(err)?;
    let end = rest.find(" @@").ok_or_else(err)?;
    let ranges = &rest[..end];
    let mut parts = ranges.split(' ');
    let old = parts.next().ok_or_else(err)?;
    let new = parts.next().ok_or_else(err)?;
    let (old_start, old_count) =
        parse_range(old.strip_prefix('-').ok_or_else(err)?).ok_or_else(err)?;
    let (new_start, new_count) =
        parse_range(new.strip_prefix('+').ok_or_else(err)?).ok_or_else(err)?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Removes the `a/`/`b/` prefix and any C-style quoting from a `---`/`+++` path.
fn strip_prefix_and_unquote(raw: &str) -> String {
    // git may append a tab and a timestamp in some modes; the path never contains a tab unquoted.
    let raw = raw.split('\t').next().unwrap_or(raw);
    let unquoted = unquote(raw);
    if unquoted == "/dev/null" {
        return unquoted;
    }
    strip_ab_prefix(&unquoted)
}

fn strip_ab_prefix(p: &str) -> String {
    for prefix in ["a/", "b/"] {
        if let Some(rest) = p.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    p.to_string()
}

/// Parses `diff --git a/P b/P` where both paths are equal (plumbing never emits renames here).
fn path_from_diff_git_line(line: &[u8]) -> Result<String, PatchError> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end_matches(['\n', '\r']);
    let rest = text
        .strip_prefix("diff --git ")
        .ok_or_else(|| PatchError::Path(text.to_string()))?;
    if rest.starts_with('"') {
        // "a/x y" "b/x y"
        let (first, _) = take_quoted(rest).ok_or_else(|| PatchError::Path(text.to_string()))?;
        return Ok(strip_ab_prefix(&unquote(first)));
    }
    // a/P b/P  -> total length = 2 + |P| + 1 + 2 + |P|
    let len = rest.len();
    if len < 5 || !rest.starts_with("a/") {
        return Err(PatchError::Path(text.to_string()));
    }
    let plen = (len - 5) / 2;
    let candidate = &rest[2..2 + plen];
    if rest.get(2 + plen..) == Some(&format!(" b/{candidate}")) {
        return Ok(candidate.to_string());
    }
    // Fall back to the last " b/" occurrence.
    match rest.rfind(" b/") {
        Some(i) => Ok(rest[i + 3..].to_string()),
        None => Err(PatchError::Path(text.to_string())),
    }
}

/// Returns the quoted token (with quotes) and the remainder.
fn take_quoted(s: &str) -> Option<(&str, &str)> {
    let bytes = s.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return Some((&s[..=i], &s[i + 1..])),
            _ => i += 1,
        }
    }
    None
}

/// Undoes git's C-style path quoting (`"a/f\303\244.txt"` -> `a/fä.txt`).
pub fn unquote(s: &str) -> String {
    if !(s.len() >= 2 && s.starts_with('"') && s.ends_with('"')) {
        return s.to_string();
    }
    let inner = &s.as_bytes()[1..s.len() - 1];
    let mut out: Vec<u8> = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let b = inner[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        i += 1;
        if i >= inner.len() {
            break;
        }
        match inner[i] {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'v' => out.push(11),
            b'\\' => out.push(b'\\'),
            b'"' => out.push(b'"'),
            d @ b'0'..=b'7' => {
                let mut val = u32::from(d - b'0');
                let mut n = 1;
                while n < 3 && i + 1 < inner.len() && (b'0'..=b'7').contains(&inner[i + 1]) {
                    i += 1;
                    val = val * 8 + u32::from(inner[i] - b'0');
                    n += 1;
                }
                out.push(val as u8);
            }
            other => {
                out.push(b'\\');
                out.push(other);
            }
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_mode_line(line: &[u8]) -> bool {
    line.starts_with(b"old mode ") || line.starts_with(b"new mode ")
}

/// Assembles a patch for `file` containing only `selected` hunks (indices into `file.hunks`).
///
/// `applied` lists hunks of the same file that earlier commits already applied; the
/// index therefore no longer equals the original base for this file and the header
/// line numbers are shifted accordingly:
///
/// ```text
/// old_start' = old_start + Σ delta(h) for h in applied  with h.old_start < this.old_start
/// new_start' = old_start' + Σ delta(h) for h in selected with h before this
/// ```
pub fn build_patch(file: &FilePatch, selected: &[usize], applied: &[usize]) -> Vec<u8> {
    if file.atomic() {
        return file.raw.clone();
    }
    let mut out = Vec::new();
    for line in lines(&file.header) {
        // A mode change travels with the first commit that touches the file.
        if !applied.is_empty() && is_mode_line(line) {
            continue;
        }
        out.extend_from_slice(line);
    }
    let mut selected: Vec<usize> = selected.to_vec();
    selected.sort_unstable();
    selected.dedup();
    let mut selected_delta: i64 = 0;
    for &idx in &selected {
        let hunk = &file.hunks[idx];
        let applied_delta: i64 = applied
            .iter()
            .filter(|&&j| file.hunks[j].old_start < hunk.old_start)
            .map(|&j| file.hunks[j].delta())
            .sum();
        let old_start = (i64::from(hunk.old_start) + applied_delta).max(0) as u32;
        let new_start = (i64::from(old_start) + selected_delta).max(0) as u32;
        out.extend_from_slice(&hunk.header_with(old_start, new_start));
        out.extend_from_slice(hunk.body());
        selected_delta += hunk.delta();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_HUNKS: &[u8] = b"diff --git a/f.txt b/f.txt\nindex 92dfa21..98bda7f 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,4 +1,6 @@\n-a\n+A\n+A2\n+A3\n b\n c\n d\n@@ -7,4 +9,4 @@ fn ctx()\n g\n h\n i\n-j\n+J\n";

    #[test]
    fn parses_text_hunks_verbatim() {
        let files = parse_patch(TWO_HUNKS).unwrap();
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(f.path, "f.txt");
        assert_eq!(f.kind, FileKind::Modified);
        assert!(!f.binary && !f.atomic());
        assert_eq!(f.hunks.len(), 2);
        assert_eq!((f.hunks[0].added, f.hunks[0].removed), (3, 1));
        assert_eq!((f.hunks[1].old_start, f.hunks[1].new_start), (7, 9));
        assert_eq!(f.header, b"diff --git a/f.txt b/f.txt\nindex 92dfa21..98bda7f 100644\n--- a/f.txt\n+++ b/f.txt\n");
        // Full re-assembly is byte-identical.
        assert_eq!(build_patch(f, &[0, 1], &[]), TWO_HUNKS);
        assert_eq!(f.raw, TWO_HUNKS);
    }

    #[test]
    fn partial_patches_recompute_offsets() {
        let f = &parse_patch(TWO_HUNKS).unwrap()[0];
        // Second hunk alone against the original base: old_start unchanged, new_start = old_start.
        let second = build_patch(f, &[1], &[]);
        assert!(second.ends_with(b"@@ -7,4 +7,4 @@ fn ctx()\n g\n h\n i\n-j\n+J\n"));
        // Second hunk after the first was committed: both starts shift by +2.
        let after = build_patch(f, &[1], &[0]);
        assert!(after.ends_with(b"@@ -9,4 +9,4 @@ fn ctx()\n g\n h\n i\n-j\n+J\n"));
        // First hunk after the second was committed: unaffected.
        let first = build_patch(f, &[0], &[1]);
        assert!(first.contains_hunk(b"@@ -1,4 +1,6 @@\n"));
    }

    trait Contains {
        fn contains_hunk(&self, needle: &[u8]) -> bool;
    }
    impl Contains for Vec<u8> {
        fn contains_hunk(&self, needle: &[u8]) -> bool {
            find(self, needle).is_some()
        }
    }

    #[test]
    fn mode_lines_only_in_first_partial_patch() {
        let data = b"diff --git a/f.txt b/f.txt\nold mode 100644\nnew mode 100755\nindex 1..2\n--- a/f.txt\n+++ b/f.txt\n@@ -1,1 +1,1 @@\n-a\n+A\n@@ -9,1 +9,1 @@\n-j\n+J\n";
        let f = &parse_patch(data).unwrap()[0];
        assert_eq!(f.kind, FileKind::Modified);
        assert_eq!(f.mode_change, Some(("100644".into(), "100755".into())));
        assert!(build_patch(f, &[0], &[]).contains_hunk(b"new mode 100755\n"));
        assert!(!build_patch(f, &[1], &[0]).contains_hunk(b"new mode"));
    }

    #[test]
    fn classifies_mode_only_binary_new_deleted_submodule() {
        let data = b"diff --git a/m.txt b/m.txt\nold mode 100644\nnew mode 100755\ndiff --git a/b.dat b/b.dat\nindex bdc955b..40d36f1 100644\nGIT binary patch\nliteral 2\nJcmZQ#1^@sF00sa6\n\nliteral 2\nJcmZQz1ONa700IC2\n\ndiff --git a/n.txt b/n.txt\nnew file mode 100644\nindex 0000000..3e75765\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+new\ndiff --git a/d.txt b/d.txt\ndeleted file mode 100644\nindex 3e75765..0000000\n--- a/d.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-new\ndiff --git a/sub b/sub\nindex 6bfbea1..5a3b127 160000\n--- a/sub\n+++ b/sub\n@@ -1 +1 @@\n-Subproject commit 6bfbea1f342e5d68daa2b8956a488b26f86b9272\n+Subproject commit 5a3b127087b3ff7f922eaeda5d5d872d2a59582c\ndiff --git a/x.bin b/x.bin\nindex 1..2 100644\nBinary files a/x.bin and b/x.bin differ\n";
        let files = parse_patch(data).unwrap();
        let kinds: Vec<_> = files
            .iter()
            .map(|f| (f.path.as_str(), f.kind, f.binary))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("m.txt", FileKind::Mode, false),
                ("b.dat", FileKind::Modified, true),
                ("n.txt", FileKind::Added, false),
                ("d.txt", FileKind::Deleted, false),
                ("sub", FileKind::Submodule, false),
                ("x.bin", FileKind::Modified, true),
            ]
        );
        assert!(files.iter().all(|f| f.atomic()));
        assert_eq!(
            files[4].submodule_commits(),
            Some((
                Some("6bfbea1f342e5d68daa2b8956a488b26f86b9272".into()),
                Some("5a3b127087b3ff7f922eaeda5d5d872d2a59582c".into())
            ))
        );
        // Raw sections are verbatim and concatenate back to the input.
        let joined: Vec<u8> = files.iter().flat_map(|f| f.raw.clone()).collect();
        assert_eq!(joined, data);
    }

    #[test]
    fn merges_type_change_sections() {
        let data = b"diff --git a/f.txt b/f.txt\ndeleted file mode 100644\nindex f0f2307..0000000\n--- a/f.txt\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-l1\n-l2\ndiff --git a/f.txt b/f.txt\nnew file mode 120000\nindex 0000000..b310e29\n--- /dev/null\n+++ b/f.txt\n@@ -0,0 +1 @@\n+g.txt\n\\ No newline at end of file\n";
        let files = parse_patch(data).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].kind, FileKind::TypeChange);
        assert!(files[0].atomic());
        assert_eq!(files[0].raw, data);
    }

    #[test]
    fn keeps_crlf_and_no_newline_markers() {
        let data = b"diff --git a/c.txt b/c.txt\nindex 1..2 100644\n--- a/c.txt\n+++ b/c.txt\n@@ -1,3 +1,3 @@\n a\r\n-b\r\n+B\r\n c\r\n\\ No newline at end of file\n";
        let f = &parse_patch(data).unwrap()[0];
        assert_eq!(f.hunks.len(), 1);
        assert_eq!(build_patch(f, &[0], &[]), data);
    }

    #[test]
    fn unquotes_paths() {
        assert_eq!(unquote("\"a/f\\303\\244.txt\""), "a/fä.txt");
        assert_eq!(unquote("\"x\\ty\""), "x\ty");
        assert_eq!(unquote("plain"), "plain");
        let data =
            b"diff --git \"a/sp ace.txt\" \"b/sp ace.txt\"\nold mode 100644\nnew mode 100755\n";
        assert_eq!(parse_patch(data).unwrap()[0].path, "sp ace.txt");
        let data = b"diff --git a/sp ace.txt b/sp ace.txt\nold mode 100644\nnew mode 100755\n";
        assert_eq!(parse_patch(data).unwrap()[0].path, "sp ace.txt");
    }

    #[test]
    fn empty_input_is_empty() {
        assert!(parse_patch(b"").unwrap().is_empty());
    }
}
