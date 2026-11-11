//! Streaming Python reference index.
//!
//! Files are read, parsed and reduced to a [`facts::FactSet`] one at a time;
//! the source text and parse tree are dropped before the next file. Only the
//! export table ([`resolve::Exports`]) is resident across files. Resolution
//! then runs per file against that table and emits sites into posting lists.
//! See `docs/design/streaming-index.md` for the sizing argument.

pub mod diagnostics;
pub mod facts;
pub mod manifest;
pub mod resolve;
pub mod store;
