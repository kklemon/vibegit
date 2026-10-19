//! Parser for `git status --porcelain=v2 -z`.
//!
//! Porcelain v2 is the only status format that reports renames, unmerged
//! entries, untracked files, nested repositories and submodule state in one
//! stable, config-independent listing.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SubmoduleState {
    /// The submodule's HEAD moved relative to the superproject's index/HEAD.
    pub commit_changed: bool,
    /// Tracked files inside the submodule are modified.
    pub tracked_modified: bool,
    /// Untracked files exist inside the submodule.
    pub has_untracked: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Ordinary,
    Rename,
    Unmerged,
    Untracked,
    /// An untracked directory that contains its own `.git` (reported as `dir/`).
    NestedRepo,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub kind: EntryKind,
    pub path: String,
    /// Original path for renames and copies.
    pub orig_path: Option<String>,
    /// Index status letter (`.` when unchanged). Unmerged entries use the `u` line's XY.
    pub index: char,
    /// Worktree status letter (`.` when unchanged).
    pub worktree: char,
    pub submodule: Option<SubmoduleState>,
}

impl StatusEntry {
    pub fn is_staged(&self) -> bool {
        matches!(self.kind, EntryKind::Ordinary | EntryKind::Rename) && self.index != '.'
    }

    /// `git add -N` entries: an index entry exists, so the index no longer equals HEAD.
    pub fn is_intent_to_add(&self) -> bool {
        self.kind == EntryKind::Ordinary && self.index == '.' && self.worktree == 'A'
    }

    pub fn is_worktree_changed(&self) -> bool {
        matches!(self.kind, EntryKind::Ordinary | EntryKind::Rename) && self.worktree != '.'
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unexpected porcelain v2 status entry: {0:?}")]
pub struct StatusParseError(String);

/// Parses NUL-separated porcelain v2 output.
pub fn parse_porcelain_v2(data: &[u8]) -> Result<Vec<StatusEntry>, StatusParseError> {
    let mut entries = Vec::new();
    let mut tokens = data
        .split(|b| *b == 0)
        .filter(|t| !t.is_empty())
        .map(|t| String::from_utf8_lossy(t).into_owned());

    while let Some(token) = tokens.next() {
        let (tag, rest) = match token.split_once(' ') {
            Some(parts) => parts,
            None => return Err(StatusParseError(token.clone())),
        };
        match tag {
            "1" => {
                // XY sub mH mI mW hH hI path
                let mut fields = rest.splitn(8, ' ');
                let xy = next_field(&mut fields, &token)?;
                let sub = next_field(&mut fields, &token)?;
                for _ in 0..5 {
                    next_field(&mut fields, &token)?;
                }
                let path = next_field(&mut fields, &token)?.to_string();
                let (index, worktree) = xy_chars(xy, &token)?;
                entries.push(StatusEntry {
                    kind: EntryKind::Ordinary,
                    path,
                    orig_path: None,
                    index,
                    worktree,
                    submodule: parse_submodule(sub),
                });
            }
            "2" => {
                // XY sub mH mI mW hH hI Xscore path NUL origPath
                let mut fields = rest.splitn(9, ' ');
                let xy = next_field(&mut fields, &token)?;
                let sub = next_field(&mut fields, &token)?;
                for _ in 0..6 {
                    next_field(&mut fields, &token)?;
                }
                let path = next_field(&mut fields, &token)?.to_string();
                let orig = tokens
                    .next()
                    .ok_or_else(|| StatusParseError(token.clone()))?;
                let (index, worktree) = xy_chars(xy, &token)?;
                entries.push(StatusEntry {
                    kind: EntryKind::Rename,
                    path,
                    orig_path: Some(orig),
                    index,
                    worktree,
                    submodule: parse_submodule(sub),
                });
            }
            "u" => {
                // XY sub m1 m2 m3 mW h1 h2 h3 path
                let mut fields = rest.splitn(10, ' ');
                let xy = next_field(&mut fields, &token)?;
                let sub = next_field(&mut fields, &token)?;
                for _ in 0..7 {
                    next_field(&mut fields, &token)?;
                }
                let path = next_field(&mut fields, &token)?.to_string();
                let (index, worktree) = xy_chars(xy, &token)?;
                entries.push(StatusEntry {
                    kind: EntryKind::Unmerged,
                    path,
                    orig_path: None,
                    index,
                    worktree,
                    submodule: parse_submodule(sub),
                });
            }
            "?" => {
                let kind = if rest.ends_with('/') {
                    EntryKind::NestedRepo
                } else {
                    EntryKind::Untracked
                };
                entries.push(StatusEntry {
                    kind,
                    path: rest.to_string(),
                    orig_path: None,
                    index: '.',
                    worktree: '.',
                    submodule: None,
                });
            }
            "!" => entries.push(StatusEntry {
                kind: EntryKind::Ignored,
                path: rest.to_string(),
                orig_path: None,
                index: '.',
                worktree: '.',
                submodule: None,
            }),
            "#" => {}
            _ => return Err(StatusParseError(token.clone())),
        }
    }
    Ok(entries)
}

fn next_field<'a>(
    fields: &mut impl Iterator<Item = &'a str>,
    token: &str,
) -> Result<&'a str, StatusParseError> {
    fields
        .next()
        .ok_or_else(|| StatusParseError(token.to_string()))
}

fn xy_chars(xy: &str, token: &str) -> Result<(char, char), StatusParseError> {
    let mut chars = xy.chars();
    match (chars.next(), chars.next()) {
        (Some(x), Some(y)) => Ok((x, y)),
        _ => Err(StatusParseError(token.to_string())),
    }
}

fn parse_submodule(field: &str) -> Option<SubmoduleState> {
    let bytes = field.as_bytes();
    if bytes.len() != 4 || bytes[0] != b'S' {
        return None;
    }
    Some(SubmoduleState {
        commit_changed: bytes[1] == b'C',
        tracked_modified: bytes[2] == b'M',
        has_untracked: bytes[3] == b'U',
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z(lines: &[&str]) -> Vec<u8> {
        let mut out = Vec::new();
        for l in lines {
            out.extend_from_slice(l.as_bytes());
            out.push(0);
        }
        out
    }

    #[test]
    fn parses_ordinary_rename_unmerged_untracked() {
        let data = z(&[
            "1 .M N... 100644 100644 100644 8352675d67aed6625ece79af41c27fdb4ee2e867 8352675d67aed6625ece79af41c27fdb4ee2e867 b.dat",
            "1 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 413c710890a3e1f1b53724436aec3611a2ec6a6d .gitmodules",
            "2 R. N... 100644 100644 100644 1b2a1c578aec87508e112ab973ba9fe53cbeac2e 1b2a1c578aec87508e112ab973ba9fe53cbeac2e R100 g.txt",
            "f.txt",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflicted.txt",
            "1 AM SC.U 000000 160000 160000 0000000000000000000000000000000000000000 bbbd5a4ce1c6aaa21b8b5e6a318a74222317437b sub",
            "1 .M S.M. 160000 160000 160000 da3ec9693b9ad26b0d6018836c80bf0f6f66f523 da3ec9693b9ad26b0d6018836c80bf0f6f66f523 dirty",
            "? untracked file.txt",
            "? nested/",
            "! ignored.log",
        ]);
        let entries = parse_porcelain_v2(&data).unwrap();
        assert_eq!(entries.len(), 9);
        assert_eq!(entries[0].path, "b.dat");
        assert_eq!((entries[0].index, entries[0].worktree), ('.', 'M'));
        assert!(entries[0].is_worktree_changed() && !entries[0].is_staged());
        assert!(entries[1].is_staged());
        assert_eq!(entries[2].kind, EntryKind::Rename);
        assert_eq!(entries[2].path, "g.txt");
        assert_eq!(entries[2].orig_path.as_deref(), Some("f.txt"));
        assert_eq!(entries[3].kind, EntryKind::Unmerged);
        assert_eq!(entries[3].path, "conflicted.txt");
        assert_eq!(
            entries[4].submodule,
            Some(SubmoduleState {
                commit_changed: true,
                tracked_modified: false,
                has_untracked: true
            })
        );
        assert_eq!(
            entries[5].submodule,
            Some(SubmoduleState {
                commit_changed: false,
                tracked_modified: true,
                has_untracked: false
            })
        );
        assert_eq!(entries[6].kind, EntryKind::Untracked);
        assert_eq!(entries[6].path, "untracked file.txt");
        assert_eq!(entries[7].kind, EntryKind::NestedRepo);
        assert_eq!(entries[8].kind, EntryKind::Ignored);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_porcelain_v2(&z(&["x y"])).is_err());
    }
}
