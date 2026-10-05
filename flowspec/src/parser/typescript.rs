//! Native TypeScript/TSX parsing; original bytes are semantic authority.
use super::{ir::ParseResult, report::ParseOutput, LanguageAdapter};
use crate::error::FlowspecError;
use std::path::Path;
#[derive(Default)]
pub struct TsAdapter;
impl TsAdapter {
    pub fn new() -> Self {
        Self
    }
}
pub(crate) fn grammar_for_path(path: &Path) -> tree_sitter::Language {
    if path.extension().is_some_and(|e| e == "tsx") {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    } else {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    }
}
impl LanguageAdapter for TsAdapter {
    fn language_name(&self) -> &str {
        "typescript"
    }
    fn can_handle(&self, path: &Path) -> bool {
        matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("ts" | "tsx")
        )
    }
    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError> {
        Ok(self.parse_output(path, content)?.ir)
    }
    fn parse_output(&self, path: &Path, content: &str) -> Result<ParseOutput, FlowspecError> {
        super::javascript::parse_native(
            path,
            content,
            grammar_for_path(path),
            "typescript",
            "tree-sitter-typescript/0.23.2+tsx",
        )
    }
}
