//! VibeGit: turn a busy working tree into focused, reviewable commits.

pub mod changes;
pub mod cli;
pub mod config;
pub mod git;
pub mod llm;
pub mod openrouter;
pub mod prompts;
pub mod proposal;
pub mod rules;
pub mod tui;
pub mod update;
pub mod workflow;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DEFAULT_RULES: &str = include_str!("../assets/default.vibegitrules");
