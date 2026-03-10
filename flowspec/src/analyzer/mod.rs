// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Diagnostic detection, flow tracing, and boundary analysis.
//!
//! Each analyzer is a standalone function that queries the graph and
//! produces diagnostics. No inheritance, no trait objects. The three
//! cycle-1 patterns are: `isolated_cluster`, `data_dead_end`, and
//! `phantom_dependency`.

pub mod diagnostic;
pub mod patterns;
