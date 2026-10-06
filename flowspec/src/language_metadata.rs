//! Language identity, extension and grammar metadata below parsing and orchestration.
use std::path::Path;
#[derive(Clone, Copy)]
pub struct BuiltinLanguage {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub grammar_version: &'static str,
}
pub const BUILTIN_LANGUAGES: [BuiltinLanguage; 4] = [
    BuiltinLanguage {
        id: "python",
        aliases: &["py"],
        extensions: &["py"],
        grammar_version: "tree-sitter-python/0.25.0",
    },
    BuiltinLanguage {
        id: "javascript",
        aliases: &["js"],
        extensions: &["js", "jsx", "mjs", "cjs"],
        grammar_version: "tree-sitter-javascript/0.25.0",
    },
    BuiltinLanguage {
        id: "typescript",
        aliases: &["ts"],
        extensions: &["ts", "tsx"],
        grammar_version: "tree-sitter-typescript/0.23.2+tsx",
    },
    BuiltinLanguage {
        id: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        grammar_version: "tree-sitter-rust/0.24.0",
    },
];
/// Minimal source classification interface; neither graph nor naming requires callbacks.
pub trait LanguageLookup {
    fn language_id_for_path(&self, path: &Path) -> Option<&str>;
}
pub fn builtin_language_for_path(path: &Path) -> Option<&'static BuiltinLanguage> {
    let extension = path.extension()?.to_str()?;
    BUILTIN_LANGUAGES
        .iter()
        .find(|d| d.extensions.contains(&extension))
}
pub fn language_id_for_path(path: &Path) -> Option<&'static str> {
    builtin_language_for_path(path).map(|d| d.id)
}
pub fn language_names() -> String {
    BUILTIN_LANGUAGES
        .iter()
        .map(|d| d.id)
        .collect::<Vec<_>>()
        .join(", ")
}
