pub mod patch;
pub mod repo;
pub mod runner;
pub mod status;

pub use patch::{build_patch, parse_patch, FileKind, FilePatch, Hunk};
pub use repo::{CommitInfo, CommitMode, GitRepo, InProgress, RepoState};
pub use runner::{GitError, GitOutput, GitResult, GitRunner};
pub use status::{parse_porcelain_v2, EntryKind, StatusEntry, SubmoduleState};
