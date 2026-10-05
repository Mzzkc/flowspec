//! Qualified module identities for production import binding. A language name
//! alone is insufficient: packages/crates may also contain same-name modules.
use crate::registry::Registry;
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
    pub fn new(root: &Path, files: &[PathBuf], registry: &Registry) -> Self {
        let mut index = Self {
            entries: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        let mut groups: BTreeMap<(String, PathBuf), Vec<PathBuf>> = BTreeMap::new();
        for file in files {
            let Some(language) = registry.language_for_path(file) else {
                continue;
            };
            let package = package_anchor(root, file, &language.id);
            index
                .files
                .insert(file.clone(), (language.id.clone(), package.clone()));
            groups
                .entry((language.id.clone(), package))
                .or_default()
                .push(file.clone());
        }
        for ((language, package), files) in groups {
            let refs: Vec<_> = files.iter().collect();
            let prefix = if language == "python" {
                crate::find_common_prefix(&refs)
            } else {
                crate::find_js_common_prefix(&refs)
            };
            let crate_roots = if language == "rust" {
                crate::find_rust_crate_roots(&refs)
            } else {
                Vec::new()
            };
            for path in files {
                let anchor = if language == "rust" {
                    crate::closest_rust_crate_root(&path, &crate_roots).unwrap_or(&prefix)
                } else {
                    &prefix
                };
                let relative = path.strip_prefix(anchor).unwrap_or(&path);
                let module = match language.as_str() {
                    "python" => crate::path_to_module_name(relative),
                    "javascript" | "typescript" => crate::js_path_to_module_key(relative),
                    "rust" => crate::rust_path_to_module_key(relative),
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
            // including same-stem JS/TS rivals. These keys are never module names.
            for (path, (target_language, target_package)) in &self.files {
                if target_package == package
                    && matches!(target_language.as_str(), "javascript" | "typescript")
                {
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
