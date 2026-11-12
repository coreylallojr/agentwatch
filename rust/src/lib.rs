//! Agent Watch: a local, read-only, zero-token dashboard for Claude Code sessions and subagents.
//! The library holds everything; src/main.rs is the CLI.
pub mod cli;
pub mod clock;
pub mod collection;
pub mod js;
pub mod mcp;
pub mod parse;
pub mod server;
pub mod store;
pub mod sys;
