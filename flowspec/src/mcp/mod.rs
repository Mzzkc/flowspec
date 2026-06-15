//! MCP (Model Context Protocol) server for Flowspec.
//!
//! Exposes Flowspec's analysis capabilities as MCP tools, communicable via
//! JSON-RPC 2.0 over stdio. Behind the `mcp` feature flag.
//!
//! Enable with `cargo build --features mcp`. Run with `flowspec mcp`.

#[cfg(feature = "mcp")]
pub mod server;
