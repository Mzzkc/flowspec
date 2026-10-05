//! Root-scoped local MCP 2025-06-18 over stdio.
//! The packaged CLI enables this feature and serves with `flowspec mcp serve --root PATH`.
//! Tools return the full AnalysisReport as structured content and equal serialized JSON text.
pub mod server;
