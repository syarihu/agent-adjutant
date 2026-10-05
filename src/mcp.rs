//! The stdio MCP server now lives in `transport::mcp`; this is the entry `lib.rs` calls it by.

pub use crate::transport::mcp::{install, run_server, uninstall};
