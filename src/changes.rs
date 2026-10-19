//! The set of change units the model assigns to commits.
//!
//! A change unit is one hunk of a modified text file, or a whole file for
//! anything that cannot be split (binary, new, deleted, mode-only, type change,
//! submodule). Units are numbered from 1 in the order they are shown to the model.

use std::collections::{BTreeMap, HashSet};

use crate::git::{build_patch, FileKind, FilePatch};
use crate::proposal::ProposalResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Tracked,
    Untracked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub id: u32,
    /// Index into `ChangeSet::files`.
    pub file: usize,
    /// Index into the file's hunks, or `None` when the whole file is the unit.
    pub hunk: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub patch: FilePatch,
    pub origin: Origin,
    /// Shown instead of the content (oversize files, submodule summaries).
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSummary {
    pub path: String,
    pub kind: FileKind,
    pub binary: bool,
    pub added: u32,
    pub removed: u32,
}

impl FileSummary {
    /// One-letter status as `git status` would show it.
    pub fn letter(&self) -> char {
        match self.kind {
            FileKind::Added => 'A',
            FileKind::Deleted => 'D',
            FileKind::TypeChange => 'T',
            FileKind::Submodule => 'S',
            FileKind::Mode | FileKind::Modified => 'M',
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChangeSet {
    pub files: Vec<FileEntry>,
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
    applied: HashSet<u32>,
}

#[derive(Debug, thiserror::Error)]
pub enum ChangeError {
    #[error("change ID {0} does not exist")]
    UnknownId(u32),
    #[error("change ID {0} was already committed")]
    AlreadyApplied(u32),
    #[error("{0}")]
    Invalid(String),
}

impl ChangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_file(&mut self, patch: FilePatch, origin: Origin, placeholder: Option<String>) {
        let file_index = self.files.len();
        if patch.atomic() || placeholder.is_some() {
            let id = self.changes.len() as u32 + 1;
            self.changes.push(Change {
                id,
                file: file_index,
                hunk: None,
            });
        } else {
            for hunk in 0..patch.hunks.len() {
                let id = self.changes.len() as u32 + 1;
                self.changes.push(Change {
                    id,
                    file: file_index,
                    hunk: Some(hunk),
                });
            }
        }
        self.files.push(FileEntry {
            patch,
            origin,
            placeholder,
        });
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn ids(&self) -> Vec<u32> {
        self.changes.iter().map(|c| c.id).collect()
    }

    pub fn change(&self, id: u32) -> Option<&Change> {
        self.changes.iter().find(|c| c.id == id)
    }

    pub fn is_applied(&self, id: u32) -> bool {
        self.applied.contains(&id)
    }

    pub fn pending_ids(&self) -> Vec<u32> {
        self.changes
            .iter()
            .map(|c| c.id)
            .filter(|id| !self.applied.contains(id))
            .collect()
    }

    pub fn mark_applied(&mut self, ids: &[u32]) {
        self.applied.extend(ids.iter().copied());
    }

    /// Validates a model response: every id exists, is used once, and nothing is left out.
    pub fn validate(&self, result: &ProposalResult) -> Result<(), ChangeError> {
        let mut seen: HashSet<u32> = HashSet::new();
        let groups = result
            .commit_proposals
            .iter()
            .map(|p| (&p.change_ids, "proposal"))
            .chain(
                result
                    .excluded_groups
                    .iter()
                    .map(|g| (&g.change_ids, "excluded group")),
            );
        for (ids, what) in groups {
            if ids.is_empty() {
                return Err(ChangeError::Invalid(format!(
                    "a {what} contains no change IDs"
                )));
            }
            for id in ids {
                if self.change(*id).is_none() {
                    return Err(ChangeError::UnknownId(*id));
                }
                if !seen.insert(*id) {
                    return Err(ChangeError::Invalid(format!(
                        "change ID {id} is included in more than one proposal or exclusion group"
                    )));
                }
            }
        }
        for p in &result.commit_proposals {
            if p.commit_message.trim().is_empty() {
                return Err(ChangeError::Invalid(format!(
                    "the proposal for change IDs {:?} has an empty commit message",
                    p.change_ids
                )));
            }
        }
        let missing: Vec<u32> = self
            .changes
            .iter()
            .map(|c| c.id)
            .filter(|id| !seen.contains(id) && !self.applied.contains(id))
            .collect();
        if !missing.is_empty() {
            let list: Vec<String> = missing.iter().map(|i| i.to_string()).collect();
            return Err(ChangeError::Invalid(format!(
                "changes not covered by the response: {}",
                list.join(", ")
            )));
        }
        Ok(())
    }

    /// Groups ids by file, in file order; the hunk list is empty for whole-file units.
    fn group_by_file(&self, ids: &[u32]) -> Result<BTreeMap<usize, Vec<usize>>, ChangeError> {
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for id in ids {
            let change = self.change(*id).ok_or(ChangeError::UnknownId(*id))?;
            let entry = groups.entry(change.file).or_default();
            if let Some(h) = change.hunk {
                entry.push(h);
            }
        }
        Ok(groups)
    }

    /// Assembles the patch that stages exactly these change units on top of the
    /// current index, accounting for hunks of the same files that were already committed.
    pub fn patch_for(&self, ids: &[u32]) -> Result<Vec<u8>, ChangeError> {
        for id in ids {
            if self.applied.contains(id) {
                return Err(ChangeError::AlreadyApplied(*id));
            }
        }
        let mut out = Vec::new();
        for (file_index, hunks) in self.group_by_file(ids)? {
            let file = &self.files[file_index].patch;
            let applied: Vec<usize> = self
                .changes
                .iter()
                .filter(|c| c.file == file_index && self.applied.contains(&c.id))
                .filter_map(|c| c.hunk)
                .collect();
            out.extend_from_slice(&build_patch(file, &hunks, &applied));
        }
        Ok(out)
    }

    /// Human-readable diff of these change units (no offset bookkeeping), for display.
    pub fn preview(&self, ids: &[u32]) -> String {
        let mut out = Vec::new();
        if let Ok(groups) = self.group_by_file(ids) {
            for (file_index, mut hunks) in groups {
                let file = &self.files[file_index].patch;
                if file.atomic() {
                    out.extend_from_slice(&file.raw);
                } else {
                    hunks.sort_unstable();
                    out.extend_from_slice(&file.header);
                    for h in hunks {
                        out.extend_from_slice(&file.hunks[h].text);
                    }
                }
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    pub fn files_for(&self, ids: &[u32]) -> Vec<FileSummary> {
        let mut out = Vec::new();
        if let Ok(groups) = self.group_by_file(ids) {
            for (file_index, hunks) in groups {
                let file = &self.files[file_index].patch;
                let (added, removed) = if file.atomic() {
                    (file.added(), file.removed())
                } else {
                    hunks
                        .iter()
                        .map(|h| (file.hunks[*h].added, file.hunks[*h].removed))
                        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1))
                };
                out.push(FileSummary {
                    path: file.path.clone(),
                    kind: file.kind,
                    binary: file.binary,
                    added,
                    removed,
                });
            }
        }
        out
    }

    pub fn paths_for(&self, ids: &[u32]) -> Vec<String> {
        self.files_for(ids).into_iter().map(|f| f.path).collect()
    }

    /// Renders every pending change unit for the model.
    pub fn render_for_model(&self, truncate: Option<usize>) -> String {
        let mut tracked = Vec::new();
        let mut untracked = Vec::new();
        for (file_index, entry) in self.files.iter().enumerate() {
            let rendered = self.render_file(file_index, truncate);
            if rendered.is_empty() {
                continue;
            }
            match entry.origin {
                Origin::Tracked => tracked.push(rendered),
                Origin::Untracked => untracked.push(rendered),
            }
        }
        let mut out = String::new();
        if !tracked.is_empty() {
            out.push_str("Changed Files:\n\n");
            out.push_str(&tracked.join("\n\n"));
        }
        if !untracked.is_empty() {
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str("Untracked Files:\n\n");
            out.push_str(&untracked.join("\n\n"));
        }
        out
    }

    fn render_file(&self, file_index: usize, truncate: Option<usize>) -> String {
        let entry = &self.files[file_index];
        let file = &entry.patch;
        let units: Vec<&Change> = self
            .changes
            .iter()
            .filter(|c| c.file == file_index && !self.applied.contains(&c.id))
            .collect();
        if units.is_empty() {
            return String::new();
        }
        let mut lines: Vec<String> = vec![summary_line(file, entry.origin)];
        let header_text = String::from_utf8_lossy(&file.header);
        if units.len() == 1 && units[0].hunk.is_none() {
            lines.push(format!("# Change ID: {}", units[0].id));
            if let Some(p) = &entry.placeholder {
                lines.push(p.clone());
            } else {
                lines.extend(header_lines(&header_text));
                if file.binary {
                    lines.push("<binary data placeholder>".to_string());
                } else {
                    for hunk in &file.hunks {
                        lines.extend(body_lines(&hunk.text));
                    }
                }
            }
        } else {
            lines.extend(header_lines(&header_text));
            for unit in units {
                if let Some(h) = unit.hunk {
                    lines.push(format!("# Change ID: {}", unit.id));
                    lines.extend(body_lines(&file.hunks[h].text));
                }
            }
        }
        let lines: Vec<String> = match truncate {
            Some(n) if n > 0 => lines.into_iter().map(|l| truncate_line(&l, n)).collect(),
            _ => lines,
        };
        lines.join("\n")
    }
}

fn summary_line(file: &FilePatch, origin: Origin) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (origin, file.kind) {
        (Origin::Untracked, _) => parts.push("new file".into()),
        (_, kind) => parts.push(kind.to_string()),
    }
    if file.binary {
        parts.push("binary".into());
    }
    if let Some((old, new)) = &file.mode_change {
        parts.push(format!("mode {old} -> {new}"));
    }
    if !file.binary && !file.hunks.is_empty() && file.kind != FileKind::Submodule {
        parts.push(format!("+{} -{}", file.added(), file.removed()));
    }
    format!("### {} ({})", file.path, parts.join(", "))
}

fn header_lines(header: &str) -> Vec<String> {
    header
        .lines()
        .filter(|l| !l.starts_with("diff --git ") && !l.starts_with("index "))
        .map(|l| l.to_string())
        .collect()
}

fn body_lines(text: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(text)
        .lines()
        .map(|l| l.to_string())
        .collect()
}

fn truncate_line(line: &str, max_chars: usize) -> String {
    if line.chars().count() <= max_chars {
        return line.to_string();
    }
    let kept: String = line.chars().take(max_chars).collect();
    format!("{kept}...")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::parse_patch;
    use crate::proposal::{CommitProposal, ExcludedGroup};

    fn sample() -> ChangeSet {
        let tracked = b"diff --git a/f.txt b/f.txt\nindex 1..2 100644\n--- a/f.txt\n+++ b/f.txt\n@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -9,2 +9,2 @@\n-j\n+J\n k\ndiff --git a/m.txt b/m.txt\nold mode 100644\nnew mode 100755\n";
        let untracked = b"diff --git a/n.txt b/n.txt\nnew file mode 100644\nindex 0000000..3e75765\n--- /dev/null\n+++ b/n.txt\n@@ -0,0 +1 @@\n+new\n";
        let mut cs = ChangeSet::new();
        for f in parse_patch(tracked).unwrap() {
            cs.add_file(f, Origin::Tracked, None);
        }
        for f in parse_patch(untracked).unwrap() {
            cs.add_file(f, Origin::Untracked, None);
        }
        cs
    }

    fn proposal(ids: &[u32]) -> CommitProposal {
        CommitProposal {
            explanation: "x".into(),
            commit_message: "msg".into(),
            change_ids: ids.to_vec(),
        }
    }

    #[test]
    fn numbers_units_and_renders() {
        let cs = sample();
        assert_eq!(cs.ids(), vec![1, 2, 3, 4]);
        let text = cs.render_for_model(Some(240));
        assert!(text.starts_with("Changed Files:\n\n### f.txt (modified, +2 -2)\n--- a/f.txt\n+++ b/f.txt\n# Change ID: 1\n@@ -1,2 +1,2 @@"), "{text}");
        assert!(text.contains("### m.txt (mode change, mode 100644 -> 100755)\n# Change ID: 3\nold mode 100644\nnew mode 100755"), "{text}");
        assert!(
            text.contains("Untracked Files:\n\n### n.txt (new file, +1 -0)\n# Change ID: 4\n"),
            "{text}"
        );
        let letters: Vec<char> = cs.files_for(&[1, 4]).iter().map(|f| f.letter()).collect();
        assert_eq!(letters, vec!['M', 'A']);
    }

    #[test]
    fn validates_coverage_and_duplicates() {
        let cs = sample();
        let ok = ProposalResult {
            commit_proposals: vec![proposal(&[1, 3]), proposal(&[2])],
            excluded_groups: vec![ExcludedGroup {
                explanation: "wip".into(),
                change_ids: vec![4],
            }],
        };
        cs.validate(&ok).unwrap();
        let missing = ProposalResult {
            commit_proposals: vec![proposal(&[1, 2, 3])],
            excluded_groups: vec![],
        };
        assert!(cs
            .validate(&missing)
            .unwrap_err()
            .to_string()
            .contains("not covered"));
        let dup = ProposalResult {
            commit_proposals: vec![proposal(&[1, 2, 3, 4]), proposal(&[4])],
            excluded_groups: vec![],
        };
        assert!(cs.validate(&dup).is_err());
        let unknown = ProposalResult {
            commit_proposals: vec![proposal(&[1, 2, 3, 4, 9])],
            excluded_groups: vec![],
        };
        assert!(matches!(
            cs.validate(&unknown),
            Err(ChangeError::UnknownId(9))
        ));
        let empty_msg = ProposalResult {
            commit_proposals: vec![CommitProposal {
                explanation: "x".into(),
                commit_message: "  ".into(),
                change_ids: vec![1, 2, 3, 4],
            }],
            excluded_groups: vec![],
        };
        assert!(cs.validate(&empty_msg).is_err());
    }

    #[test]
    fn patches_track_applied_hunks() {
        let mut cs = sample();
        let first = cs.patch_for(&[2]).unwrap();
        assert!(first.ends_with(b"@@ -9,2 +9,2 @@\n-j\n+J\n k\n"));
        cs.mark_applied(&[2]);
        assert!(matches!(
            cs.patch_for(&[2]),
            Err(ChangeError::AlreadyApplied(2))
        ));
        let second = cs.patch_for(&[1, 3]).unwrap();
        let text = String::from_utf8_lossy(&second);
        assert!(text.contains("@@ -1,2 +1,2 @@\n-a\n+A\n b\n"));
        assert!(text.contains("old mode 100644\nnew mode 100755\n"));
        assert_eq!(cs.pending_ids(), vec![1, 3, 4]);
        assert!(!cs.render_for_model(None).contains("Change ID: 2"));
    }
}
