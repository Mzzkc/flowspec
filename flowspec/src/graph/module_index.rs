//! Qualified module identities for production import binding. A language name
//! alone is insufficient: packages/crates may also contain same-name modules.
use crate::language_metadata::LanguageLookup;
use crate::module_paths;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Namespace identity for one declared module in a package/crate.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleKey {
    pub language: String,
    pub package: PathBuf,
    pub module: String,
}
/// Source-qualified index. Lookup always carries the importing file binding.
pub struct ModuleIndex {
    entries: BTreeMap<ModuleKey, Vec<PathBuf>>,
    files: BTreeMap<PathBuf, (String, PathBuf)>,
}
impl ModuleIndex {
    pub(crate) fn from_language_metadata(
        root: &Path,
        files: &[PathBuf],
        registry: &impl LanguageLookup,
    ) -> Self {
        let mut index = Self {
            entries: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        let mut groups: BTreeMap<(String, PathBuf), Vec<PathBuf>> = BTreeMap::new();
        for file in files {
            let Some(language) = registry.language_id_for_path(file) else {
                continue;
            };
            let package = package_anchor(root, file, language);
            index
                .files
                .insert(file.clone(), (language.to_string(), package.clone()));
            groups
                .entry((language.to_string(), package))
                .or_default()
                .push(file.clone());
        }
        for ((language, package), files) in groups {
            let refs: Vec<_> = files.iter().collect();
            let prefix = if language == "python" {
                module_paths::find_common_prefix(&refs)
            } else {
                module_paths::find_js_common_prefix(&refs)
            };
            let crate_roots = if language == "rust" {
                module_paths::find_rust_crate_roots(&refs)
            } else {
                Vec::new()
            };
            for path in files {
                let anchor = if language == "rust" {
                    module_paths::closest_rust_crate_root(&path, &crate_roots).unwrap_or(&prefix)
                } else {
                    &prefix
                };
                let relative = path.strip_prefix(anchor).unwrap_or(&path);
                let module = match language.as_str() {
                    "python" => module_paths::path_to_module_name(relative),
                    "javascript" | "typescript" => module_paths::js_path_to_module_key(relative),
                    "rust" => module_paths::rust_path_to_module_key(relative),
                    _ => path.to_string_lossy().into_owned(),
                };
                if !module.is_empty() {
                    index
                        .entries
                        .entry(ModuleKey {
                            language: language.clone(),
                            package: package.clone(),
                            module,
                        })
                        .or_default()
                        .push(path);
                }
            }
        }
        index
    }
    /// Resolver view for one actual import binding. JS/TS interoperation is
    /// admitted only for relative path resolution by the resolver below.
    pub(crate) fn binding_view(&self, importing_file: &Path) -> HashMap<String, PathBuf> {
        let Some((language, package)) = self.files.get(importing_file) else {
            return HashMap::new();
        };
        let mut candidates: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for (key, paths) in &self.entries {
            if &key.package == package && &key.language == language {
                candidates
                    .entry(key.module.clone())
                    .or_default()
                    .extend(paths.clone());
            }
        }
        let mut view: HashMap<String, PathBuf> = candidates
            .into_iter()
            .filter_map(|(key, paths)| {
                if paths.len() == 1 {
                    Some((key, paths[0].clone()))
                } else {
                    None
                }
            })
            .collect();
        if matches!(language.as_str(), "javascript" | "typescript") {
            // Preserve all actual source paths for relative binding resolution,
            // including cross-package targets and same-stem JS/TS rivals. These
            // keys are never module names; the relative resolver matches actual paths.
            for (path, (target_language, _)) in &self.files {
                if matches!(target_language.as_str(), "javascript" | "typescript") {
                    view.insert(format!("@source-path:{}", path.display()), path.clone());
                }
            }
        }
        view
    }
}
fn package_anchor(root: &Path, file: &Path, language: &str) -> PathBuf {
    let root = if root.is_file() {
        root.parent().unwrap_or(root)
    } else {
        root
    };
    let markers: &[&str] = match language {
        "rust" => &["Cargo.toml"],
        "python" => &["pyproject.toml", "setup.py"],
        "javascript" | "typescript" => &["package.json"],
        _ => &[],
    };
    for parent in file.ancestors().skip(1) {
        if markers.iter().any(|marker| parent.join(marker).is_file()) {
            return parent.to_path_buf();
        }
        if parent == root {
            break;
        }
    }
    root.to_path_buf()
}

#[cfg(test)]
mod metadata_boundary_tests {
    use super::*;

    struct MetadataOnly;
    impl LanguageLookup for MetadataOnly {
        fn language_id_for_path(&self, path: &Path) -> Option<&str> {
            match path.extension()?.to_str()? {
                "py" => Some("python"),
                "mjs" => Some("javascript"),
                _ => None,
            }
        }
    }

    #[test]
    fn source_module_binding_needs_metadata_without_any_callback_registry() {
        let root = Path::new("/metadata-only");
        let python = root.join("unit.py");
        let javascript = root.join("unit.mjs");
        let ignored = root.join("unit.unknown");
        let index = ModuleIndex::from_language_metadata(
            root,
            &[python.clone(), javascript.clone(), ignored.clone()],
            &MetadataOnly,
        );
        assert_eq!(index.binding_view(&python).get("unit"), Some(&python));
        assert_eq!(
            index.binding_view(&javascript).get("unit"),
            Some(&javascript)
        );
        assert!(!index.files.contains_key(&ignored));
        assert_eq!(index.files.len(), 2);
    }
}
