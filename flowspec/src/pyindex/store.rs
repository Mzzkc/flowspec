//! The on-disk index and the streaming build that fills it.
//!
//! Layout under `<root>/.flowspec/index/`:
//!
//! ```text
//! facts/<sha256>.bin            one FactSet per file; the hash covers the
//!                               file's root-relative path and its bytes
//! exports.bin                   Vec<FileExports>, one per indexed file
//! sites.bin                     Vec<Site> sorted by target
//! unknown.bin                   Vec<UnknownCall> sorted by member name
//! losses.bin                    Vec<Loss> sorted by file
//! manifest.bin                  files, hashes, import closure, version
//! ```
//!
//! Build reads each file once, extracts facts, serialises them, keeps only the
//! file's exports, and drops the source and tree before the next file. After
//! all files are read the export table is built and each file's facts are
//! re-read from disk and resolved, one at a time. Unchanged files (same byte
//! hash) are not re-parsed on a later build, and only files whose imports
//! name a changed module are re-resolved.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tree_sitter::Parser;

use super::facts::{self, DefKind, FactSet, SiteKind};
use super::resolve::{self, Exports, FileExports, GId, Loss, Site, UnknownCall};
use crate::error::FlowspecError;

const INDEX_VERSION: u32 = 6;

/// Per-file record in the manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Root-relative path with `/` separators.
    pub rel: String,
    /// SHA-256 of the root-relative path and the file bytes, hex. A fact set
    /// records its own path and module, so two files with identical content
    /// in different directories must not share one.
    pub hash: String,
    /// Modules this file imports (dotted), for incremental re-resolution.
    pub imports: Vec<String>,
    /// Why the file has no facts, if so.
    pub skipped: Option<String>,
    /// Whether tree-sitter reported syntax errors.
    pub has_errors: bool,
    /// Call expressions seen.
    pub call_count: u32,
}

/// The manifest of an index.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexManifest {
    /// Format version.
    pub version: u32,
    /// Flowspec version that wrote it.
    pub flowspec_version: String,
    /// Files, in file-id order.
    pub files: Vec<FileEntry>,
}

/// Counts describing a build.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct BuildStats {
    /// Files in the index.
    pub files: usize,
    /// Files parsed in this build (others were unchanged).
    pub parsed: usize,
    /// Files resolved in this build.
    pub resolved: usize,
    /// Files skipped (unreadable, unparsable, too deep).
    pub skipped: usize,
    /// Files with syntax errors (indexed as far as recovered).
    pub with_errors: usize,
    /// Definitions.
    pub definitions: usize,
    /// Call expressions.
    pub call_sites: usize,
    /// Calls whose receiver could not be typed.
    pub unknown_receiver_calls: usize,
    /// Sites recorded.
    pub sites: usize,
}

/// A built or loaded index, ready for queries.
pub struct Index {
    /// The export table.
    pub exports: Exports,
    sites: Vec<Site>,
    /// Indices into `sites` ordered by owner, so "what does this definition
    /// reach" is a binary-searched slice like "what reaches it".
    by_owner: Vec<u32>,
    unknown: Vec<UnknownCall>,
    losses: Vec<Loss>,
    manifest: IndexManifest,
    /// Counts from the last build (zeros when loaded without building).
    pub stats: BuildStats,
    dir: PathBuf,
    entity_ids: std::sync::OnceLock<HashMap<GId, String>>,
}

fn io(path: &Path, e: std::io::Error) -> FlowspecError {
    FlowspecError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

fn cache_err(path: &Path, reason: String) -> FlowspecError {
    FlowspecError::Cache {
        path: path.to_path_buf(),
        reason,
    }
}

/// Largest cache file the store will decode. A `.flowspec/index` that came
/// with a checkout is untrusted input; a length prefix in it must not be
/// able to request an allocation larger than any index flowspec writes.
const MAX_CACHE_BYTES: u64 = 256 << 20;

/// Length of the authentication tag that prefixes every cache file.
const TAG_LEN: usize = 32;

/// The per-user cache key: 32 random bytes kept outside any repository
/// (`$FLOWSPEC_CACHE_KEY`, else `$XDG_CACHE_HOME/flowspec/index.key`, else
/// `~/.cache/flowspec/index.key`), created mode 0600 on first use.
///
/// Every cache file is tagged with SHA-256 over this key, the file's role
/// and its payload. An index that arrives with a checkout, or that another
/// user wrote, carries no valid tag and is rebuilt from source; nothing this
/// user did not write is ever decoded. Without a usable key the store
/// still works but trusts nothing on disk: every run is a cold build.
fn cache_key() -> Option<&'static [u8; 32]> {
    use std::sync::OnceLock;
    static KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    KEY.get_or_init(|| {
        let path = std::env::var_os("FLOWSPEC_CACHE_KEY")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("XDG_CACHE_HOME")
                    .map(PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
                    .map(|c| c.join("flowspec").join("index.key"))
            })?;
        if let Ok(bytes) = fs::read(&path) {
            if bytes.len() == 32 {
                let mut k = [0u8; 32];
                k.copy_from_slice(&bytes);
                return Some(k);
            }
        }
        let mut k = [0u8; 32];
        getrandom::fill(&mut k).ok()?;
        let dir = path.parent()?;
        fs::create_dir_all(dir).ok()?;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        match opts.open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(&k).ok()?;
                Some(k)
            }
            // Another process created it first: read theirs.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let bytes = fs::read(&path).ok()?;
                (bytes.len() == 32).then(|| {
                    let mut k = [0u8; 32];
                    k.copy_from_slice(&bytes);
                    k
                })
            }
            Err(_) => None,
        }
    })
    .as_ref()
}

fn tag(key: &[u8; 32], role: &str, payload: &[u8]) -> [u8; TAG_LEN] {
    let mut h = Sha256::new();
    h.update(key);
    h.update([0u8]);
    h.update(role.as_bytes());
    h.update([0u8]);
    h.update((payload.len() as u64).to_le_bytes());
    h.update(payload);
    h.finalize().into()
}

/// The role a cache file plays: its file name, so a tag made for one file
/// is not valid for another.
fn role_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Write `value` to `path` as `tag || payload`, through a temporary file
/// created exclusively (`O_EXCL`, and `O_NOFOLLOW` on unix) and unique to
/// this process and call, so a planted symlink is never followed and two
/// concurrent builds never rename each other's half-written file.
fn write_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), FlowspecError> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let payload = bincode::serde::encode_to_vec(value, bincode::config::standard())
        .map_err(|e| cache_err(path, format!("encode: {e}")))?;
    let Some(key) = cache_key() else {
        // No key: nothing written now could be trusted later.
        return Ok(());
    };
    let mut bytes = Vec::with_capacity(TAG_LEN + payload.len());
    bytes.extend_from_slice(&tag(key, &role_of(path), &payload));
    bytes.extend_from_slice(&payload);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp{}-{seq}", std::process::id()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc_o_nofollow());
    }
    let written = opts.open(&tmp).and_then(|mut f| {
        use std::io::Write;
        f.write_all(&bytes)
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(io(&tmp, e));
    }
    // rename replaces a planted symlink at `path` itself; it never writes
    // through one.
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        io(path, e)
    })
}

#[cfg(unix)]
fn libc_o_nofollow() -> i32 {
    // O_NOFOLLOW on Linux and the BSDs; the value is part of each platform ABI.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        0o400000
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        0x0100
    }
}

/// Read and authenticate a cache file. A file with no valid tag for this
/// user's key is reported as a cache error, which every caller treats as
/// "absent": the data is rebuilt from source.
fn read_bin<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, FlowspecError> {
    let Some(key) = cache_key() else {
        return Err(cache_err(
            path,
            "no cache key; cache not trusted".to_string(),
        ));
    };
    let meta = fs::symlink_metadata(path).map_err(|e| io(path, e))?;
    if !meta.is_file() {
        return Err(cache_err(path, "not a regular file".to_string()));
    }
    if meta.len() > MAX_CACHE_BYTES + TAG_LEN as u64 {
        return Err(cache_err(
            path,
            format!("{} bytes exceeds the cache bound", meta.len()),
        ));
    }
    let bytes = fs::read(path).map_err(|e| io(path, e))?;
    if bytes.len() < TAG_LEN {
        return Err(cache_err(path, "too short to carry a tag".to_string()));
    }
    let (stored, payload) = bytes.split_at(TAG_LEN);
    let expected = tag(key, &role_of(path), payload);
    // Constant-time comparison: the tag is a secret-keyed value.
    let diff = stored
        .iter()
        .zip(expected.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if diff != 0 {
        return Err(cache_err(
            path,
            "cache tag does not match this user's key; not written by this user".to_string(),
        ));
    }
    let config = bincode::config::standard().with_limit::<{ MAX_CACHE_BYTES as usize }>();
    bincode::serde::decode_from_slice(payload, config)
        .map(|(v, _)| v)
        .map_err(|e| cache_err(path, format!("decode: {e}")))
}

/// Whether a decoded fact set is self-consistent: every scope, owner,
/// parent and class index names a definition, and parent chains end at the
/// module scope. A cache that fails this is rebuilt from source, never
/// walked.
fn facts_consistent(f: &FactSet) -> bool {
    let n = f.defs.len() as u32;
    if n == 0 || f.defs[0].parent.is_some() {
        return false;
    }
    for (i, d) in f.defs.iter().enumerate() {
        if d.parent.is_some_and(|p| p >= n) || d.class.is_some_and(|c| c >= n) {
            return false;
        }
        // Parents must point strictly backwards: no cycle is possible then.
        if i > 0 && d.parent.is_none_or(|p| p as usize >= i) {
            return false;
        }
    }
    f.imports.iter().all(|i| i.scope < n)
        && f.binds
            .iter()
            .all(|b| b.scope < n && b.class.is_none_or(|c| c < n))
        && f.refs.iter().all(|r| r.scope < n && r.owner < n)
}

/// Read a fact set, rejecting one whose indices are out of range.
fn read_facts(path: &Path) -> Result<FactSet, FlowspecError> {
    let f: FactSet = read_bin(path)?;
    if facts_consistent(&f) {
        Ok(f)
    } else {
        Err(cache_err(path, "inconsistent fact set".to_string()))
    }
}

/// Refuse an index directory component that is a symlink: a checkout could
/// point `.flowspec/index` at a directory flowspec must not write into or
/// clean up. Returns the error a caller should surface.
fn refuse_symlink(path: &Path) -> Result<(), FlowspecError> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Err(cache_err(
            path,
            "is a symlink; refusing to use it as the index directory".to_string(),
        )),
        _ => Ok(()),
    }
}

/// Indices of `sites` sorted by owner, then by the site's position in the
/// by-target order, so a forward slice lists a definition's sites in a
/// stable order.
fn owner_permutation(sites: &[Site]) -> Vec<u32> {
    let mut perm: Vec<u32> = (0..sites.len() as u32).collect();
    perm.sort_by_key(|i| (sites[*i as usize].owner, *i));
    perm
}

fn is_fact_file_name(name: &str) -> bool {
    name.len() == 68 && name.ends_with(".bin") && name[..64].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Hex SHA-256 over the root-relative path and the file bytes, so the fact
/// file of `a/x.py` is never reused for an identical `b/x.py`.
fn hash_file(rel: &str, bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(rel.as_bytes());
    hasher.update([0u8]);
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Dotted module name of a root-relative Python path, with the package
/// prefix of the root prepended.
fn module_name(prefix: &[String], rel: &str) -> (String, bool) {
    let comps: Vec<&str> = rel.split('/').collect();
    let is_package = comps.last().is_some_and(|l| *l == "__init__.py");
    let mut parts: Vec<String> = prefix.to_vec();
    for (i, c) in comps.iter().enumerate() {
        if i + 1 == comps.len() {
            if !is_package {
                parts.push(c.trim_end_matches(".py").to_string());
            }
        } else {
            parts.push((*c).to_string());
        }
    }
    (parts.join("."), is_package)
}

impl Index {
    /// Directory of the index for `root`.
    pub fn dir_for(root: &Path) -> PathBuf {
        root.join(".flowspec").join("index")
    }

    /// Build (or incrementally refresh) the index for `root` over `files`.
    ///
    /// `files` are absolute paths of the Python files to index. The index on
    /// disk, if any, is reused for unchanged files.
    pub fn build(root: &Path, files: &[PathBuf]) -> Result<Index, FlowspecError> {
        let t0 = std::time::Instant::now();
        let dir = Index::dir_for(root);
        let facts_dir = dir.join("facts");
        for p in [root.join(".flowspec"), dir.clone(), facts_dir.clone()] {
            refuse_symlink(&p)?;
        }
        fs::create_dir_all(&facts_dir).map_err(|e| io(&facts_dir, e))?;

        let previous: Option<IndexManifest> =
            read_bin(&dir.join("manifest.bin"))
                .ok()
                .filter(|m: &IndexManifest| {
                    m.version == INDEX_VERSION && m.flowspec_version == env!("CARGO_PKG_VERSION")
                });
        let prev_by_rel: HashMap<&str, &FileEntry> = previous
            .as_ref()
            .map(|m| m.files.iter().map(|f| (f.rel.as_str(), f)).collect())
            .unwrap_or_default();

        // Package prefix above the root: `src/marianne` with an `__init__.py` is `marianne`.
        let mut prefix: Vec<String> = Vec::new();
        let mut d = Some(root);
        while let Some(p) = d {
            if p.join("__init__.py").is_file() {
                if let Some(n) = p.file_name().and_then(|n| n.to_str()) {
                    prefix.insert(0, n.to_string());
                }
                d = p.parent();
            } else {
                break;
            }
        }
        let root_tail: Vec<String> = root
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(n) => n.to_str().map(str::to_string),
                _ => None,
            })
            .collect();

        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| cache_err(&dir, format!("tree-sitter python: {e}")))?;

        let mut sorted: Vec<&PathBuf> = files
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e == "py"))
            .collect();
        sorted.sort();

        let mut stats = BuildStats::default();
        let mut entries: Vec<FileEntry> = Vec::with_capacity(sorted.len());
        let mut exports: Vec<FileExports> = Vec::with_capacity(sorted.len());
        let mut changed: HashSet<usize> = HashSet::new();
        let mut buf = Vec::new();

        // Pass 1: read, hash, extract, persist facts; keep only exports.
        for path in &sorted {
            let rel_path = path.strip_prefix(root).unwrap_or(path);
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            let fi = entries.len();
            buf.clear();
            let read_ok = fs::File::open(path)
                .and_then(|mut f| f.read_to_end(&mut buf))
                .is_ok();
            if !read_ok {
                stats.skipped += 1;
                changed.insert(fi);
                entries.push(FileEntry {
                    rel: rel.clone(),
                    hash: String::new(),
                    imports: Vec::new(),
                    skipped: Some("unreadable".to_string()),
                    has_errors: false,
                    call_count: 0,
                });
                exports.push(FileExports {
                    rel,
                    ..FileExports::default()
                });
                continue;
            }
            let hash = hash_file(&rel, &buf);
            let facts_path = facts_dir.join(format!("{hash}.bin"));
            let reuse = prev_by_rel
                .get(rel.as_str())
                .filter(|e| e.hash == hash && e.skipped.is_none() && facts_path.is_file())
                .map(|e| (*e).clone());
            let (fact_set, entry) = match reuse {
                Some(prev) => match read_facts(&facts_path) {
                    Ok(f) if f.rel == rel && f.module == module_name(&prefix, &rel).0 => {
                        (Some(f), prev)
                    }
                    Ok(_) => {
                        changed.insert(fi);
                        extract_one(&mut parser, &buf, &rel, &prefix, &facts_path, &mut stats)?
                    }
                    Err(_) => {
                        changed.insert(fi);
                        extract_one(&mut parser, &buf, &rel, &prefix, &facts_path, &mut stats)?
                    }
                },
                None => {
                    changed.insert(fi);
                    extract_one(&mut parser, &buf, &rel, &prefix, &facts_path, &mut stats)?
                }
            };
            match fact_set {
                Some(f) => {
                    stats.definitions += f.defs.len().saturating_sub(1);
                    stats.call_sites += f.call_count as usize;
                    if f.has_errors {
                        stats.with_errors += 1;
                    }
                    exports.push(FileExports::from_facts(&f));
                }
                None => exports.push(FileExports {
                    rel: rel.clone(),
                    ..FileExports::default()
                }),
            }
            entries.push(entry);
            crate::budget::check("index facts")?;
        }
        stats.files = entries.len();
        tracing::debug!(
            "index pass 1 (read, hash, facts) {:?}; parsed {} reused {}",
            t0.elapsed(),
            stats.parsed,
            entries.len() - stats.parsed
        );

        // Which files must be re-resolved: changed ones, plus any file that
        // imports a module whose file changed (or appeared, or vanished).
        let changed_modules: HashSet<&str> = changed
            .iter()
            .map(|&fi| exports[fi].module.as_str())
            .collect();
        let prev_rels: HashSet<&str> = prev_by_rel.keys().copied().collect();
        let now_rels: HashSet<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
        let structure_changed = previous.is_none() || prev_rels != now_rels;
        let mut to_resolve: Vec<usize> = Vec::new();
        for (fi, e) in entries.iter().enumerate() {
            let touched = structure_changed
                || changed.contains(&fi)
                || e.imports.iter().any(|m| {
                    changed_modules.contains(m.as_str())
                        || changed_modules
                            .iter()
                            .any(|cm| m.starts_with(cm) && m[cm.len()..].starts_with('.'))
                });
            if touched {
                to_resolve.push(fi);
            }
        }

        let t1 = std::time::Instant::now();
        let ex = Exports::build(exports, root_tail);
        tracing::debug!("index export table {:?}", t1.elapsed());
        crate::budget::check("index exports")?;

        // Pass 2: resolve. Reuse prior sites for untouched files.
        let mut sites: Vec<Site> = Vec::new();
        let mut unknown: Vec<UnknownCall> = Vec::new();
        let mut losses: Vec<Loss> = Vec::new();
        if !structure_changed {
            let keep: HashSet<u32> = (0..entries.len() as u32)
                .filter(|fi| !to_resolve.contains(&(*fi as usize)))
                .collect();
            // A retained site must name definitions that exist in the
            // rebuilt export table; anything else is a stale or hostile cache.
            let in_table = |g: &GId| {
                ex.files
                    .get(g.0 as usize)
                    .is_some_and(|f| (g.1 as usize) < f.defs.len())
            };
            if let Ok(old) = read_bin::<Vec<Site>>(&dir.join("sites.bin")) {
                sites.extend(old.into_iter().filter(|s| {
                    keep.contains(&s.owner.0) && in_table(&s.owner) && in_table(&s.target)
                }));
            }
            if let Ok(old) = read_bin::<Vec<UnknownCall>>(&dir.join("unknown.bin")) {
                unknown.extend(
                    old.into_iter()
                        .filter(|u| keep.contains(&u.owner.0) && in_table(&u.owner)),
                );
            }
            if let Ok(old) = read_bin::<Vec<Loss>>(&dir.join("losses.bin")) {
                losses.extend(old.into_iter().filter(|l| keep.contains(&l.file)));
            }
        }
        for &fi in &to_resolve {
            let e = &entries[fi];
            if e.skipped.is_some() {
                continue;
            }
            let facts_path = facts_dir.join(format!("{}.bin", e.hash));
            let f = read_facts(&facts_path)?;
            let r = resolve::resolve_file(&ex, fi as u32, &f);
            sites.extend(r.sites);
            unknown.extend(r.unknown_calls);
            losses.extend(r.losses);
            stats.resolved += 1;
            crate::budget::check("index resolve")?;
        }
        tracing::debug!("index resolve {} files {:?}", stats.resolved, t1.elapsed());
        sites.sort_by_key(|s| (s.target, s.owner.0, s.line, s.column));
        unknown.sort_by(|a, b| {
            a.name
                .cmp(&b.name)
                .then(a.owner.cmp(&b.owner))
                .then(a.line.cmp(&b.line))
        });
        losses.sort_by_key(|l| (l.file, l.start, l.end));
        stats.sites = sites.len();
        stats.unknown_receiver_calls = unknown.len();

        let manifest = IndexManifest {
            version: INDEX_VERSION,
            flowspec_version: env!("CARGO_PKG_VERSION").to_string(),
            files: entries,
        };
        write_atomic(&dir.join("sites.bin"), &sites)?;
        write_atomic(&dir.join("unknown.bin"), &unknown)?;
        write_atomic(&dir.join("losses.bin"), &losses)?;
        write_atomic(&dir.join("exports.bin"), &ex.files)?;
        write_atomic(&dir.join("manifest.bin"), &manifest)?;
        tracing::debug!("index written; total {:?}", t0.elapsed());

        // Drop fact files no current file uses.
        let live: HashSet<String> = manifest
            .files
            .iter()
            .filter(|f| !f.hash.is_empty())
            .map(|f| format!("{}.bin", f.hash))
            .collect();
        // Only regular files named like a fact file are ours to delete.
        if let Ok(rd) = fs::read_dir(&facts_dir) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let regular = fs::symlink_metadata(entry.path()).is_ok_and(|m| m.is_file());
                if regular && is_fact_file_name(&name) && !live.contains(&name) {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }

        let by_owner = owner_permutation(&sites);
        Ok(Index {
            exports: ex,
            sites,
            by_owner,
            unknown,
            losses,
            manifest,
            entity_ids: std::sync::OnceLock::new(),
            stats,
            dir,
        })
    }

    /// The manifest.
    pub fn manifest(&self) -> &IndexManifest {
        &self.manifest
    }

    /// Directory the index lives in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Read a file's facts back from the store.
    pub fn facts_of(&self, fi: u32) -> Result<FactSet, FlowspecError> {
        let e = &self.manifest.files[fi as usize];
        read_facts(&self.dir.join("facts").join(format!("{}.bin", e.hash)))
    }

    /// Every site reaching `target`, certain first, plus unknown-receiver
    /// calls of the same member name as possible sites for a method.
    pub fn sites_reaching(&self, target: GId) -> Vec<Site> {
        let lo = self.sites.partition_point(|s| s.target < target);
        let hi = self.sites.partition_point(|s| s.target <= target);
        let mut out: Vec<Site> = self.sites[lo..hi].to_vec();
        let d = self.exports.def(target);
        if d.kind == DefKind::Method {
            let name = d.name.as_str();
            let ulo = self.unknown.partition_point(|u| u.name.as_str() < name);
            let uhi = self.unknown.partition_point(|u| u.name.as_str() <= name);
            for u in &self.unknown[ulo..uhi] {
                if out.iter().any(|s| s.owner == u.owner && s.line == u.line) {
                    continue;
                }
                out.push(Site {
                    target,
                    owner: u.owner,
                    line: u.line,
                    column: 0,
                    kind: SiteKind::Call,
                    certain: false,
                    why: "receiver type not proven".to_string(),
                });
            }
        }
        out.sort_by_key(|s| (!s.certain, s.owner.0, s.line));
        out
    }

    /// Every site inside `owner`.
    pub fn sites_within(&self, owner: GId) -> Vec<Site> {
        let key = |i: &u32| self.sites[*i as usize].owner;
        let lo = self.by_owner.partition_point(|i| key(i) < owner);
        let hi = self.by_owner.partition_point(|i| key(i) <= owner);
        self.by_owner[lo..hi]
            .iter()
            .map(|i| self.sites[*i as usize].clone())
            .collect()
    }

    /// All sites, sorted by target.
    pub fn all_sites(&self) -> &[Site] {
        &self.sites
    }

    /// Unknown-receiver calls, sorted by member name.
    pub fn unknown_calls(&self) -> &[UnknownCall] {
        &self.unknown
    }

    /// Every open question the resolver recorded, sorted by file and span.
    pub fn losses(&self) -> &[Loss] {
        &self.losses
    }

    /// Identifier-shaped string literals in file `fi` (a dispatch table may
    /// name a method by string).
    pub fn named_strings(&self, fi: u32) -> &HashSet<String> {
        &self.exports.files[fi as usize].named_strings
    }

    /// Manifest entity ids by definition, computed once per index.
    pub fn entity_ids(&self) -> &HashMap<GId, String> {
        self.entity_ids
            .get_or_init(|| super::manifest::entity_ids(self))
    }

    /// Resolve a user-supplied symbol to one definition.
    ///
    /// Accepts `path/file.py::Class::method`, `file.py::Class::method`,
    /// `file.py::method`, `Class.method`, `Class::method` and a bare name.
    pub fn resolve_symbol(&self, query: &str) -> Result<GId, FlowspecError> {
        let (file_part, sym_part) = match query.find("::") {
            Some(i) if query[..i].ends_with(".py") => (Some(&query[..i]), &query[i + 2..]),
            _ => (None, query),
        };
        let sym = sym_part.replace('.', "::");
        let file_matches = |rel: &str| match file_part {
            None => true,
            Some(p) => rel == p || rel.ends_with(&format!("/{p}")),
        };
        let mut exact = Vec::new();
        let mut suffix = Vec::new();
        for (fi, f) in self.exports.files.iter().enumerate() {
            if !file_matches(&f.rel) {
                continue;
            }
            for (di, d) in f.defs.iter().enumerate() {
                if d.kind == DefKind::Module {
                    continue;
                }
                if d.qual == sym {
                    exact.push((fi as u32, di as u32));
                } else if d.qual.ends_with(&format!("::{sym}")) {
                    suffix.push((fi as u32, di as u32));
                }
            }
        }
        let pool = if exact.is_empty() { suffix } else { exact };
        match pool.len() {
            1 => Ok(pool[0]),
            0 => {
                let mut msg = format!("Symbol '{query}' not found.");
                let in_file: Vec<String> = match file_part {
                    Some(_) => self
                        .exports
                        .files
                        .iter()
                        .enumerate()
                        .filter(|(_, f)| file_matches(&f.rel))
                        .flat_map(|(fi, f)| {
                            f.defs
                                .iter()
                                .enumerate()
                                .filter(|(_, d)| d.kind != DefKind::Module)
                                .map(move |(di, _)| (fi as u32, di as u32))
                        })
                        .map(|g| self.exports.entity_id(g))
                        .collect(),
                    None => Vec::new(),
                };
                let last = sym.rsplit("::").next().unwrap_or(&sym).to_string();
                if !in_file.is_empty() {
                    msg.push_str(&format!(
                        " Available in this file: {}",
                        list_capped(&in_file, 40)
                    ));
                } else {
                    let near: Vec<String> = self
                        .exports
                        .files
                        .iter()
                        .enumerate()
                        .flat_map(|(fi, f)| {
                            f.defs
                                .iter()
                                .enumerate()
                                .filter(|(_, d)| d.kind != DefKind::Module && d.name == last)
                                .map(move |(di, _)| (fi as u32, di as u32))
                        })
                        .map(|g| self.exports.entity_id(g))
                        .collect();
                    if near.is_empty() {
                        msg.push_str(&format!(
                            " No Python definition named '{last}' in {} file(s) searched. Run `flowspec analyze` to see available entities.",
                            self.exports.file_count()
                        ));
                    } else {
                        msg.push_str(&format!(" Did you mean: {}", list_capped(&near, 20)));
                    }
                }
                Err(FlowspecError::SymbolNotFound(msg))
            }
            _ => {
                let names: Vec<String> = pool.iter().map(|g| self.exports.entity_id(*g)).collect();
                Err(FlowspecError::SymbolNotFound(format!(
                    "Symbol '{query}' matches multiple entities: {}. Use a qualified name to disambiguate.",
                    list_capped(&names, 20)
                )))
            }
        }
    }

    /// Callers reachable through certain call sites, breadth-first; one
    /// shortest path per caller.
    pub fn transitive_callers(&self, target: GId, max_depth: usize) -> Vec<Vec<GId>> {
        self.bfs(target, max_depth, true)
    }

    /// Callees reachable through certain call sites, breadth-first.
    pub fn transitive_callees(&self, target: GId, max_depth: usize) -> Vec<Vec<GId>> {
        self.bfs(target, max_depth, false)
    }

    fn bfs(&self, target: GId, max_depth: usize, backward: bool) -> Vec<Vec<GId>> {
        use std::collections::VecDeque;
        let mut parent: HashMap<GId, GId> = HashMap::new();
        let mut order: Vec<GId> = Vec::new();
        let mut queue: VecDeque<(GId, usize)> = VecDeque::from([(target, 0)]);
        let mut seen: HashSet<GId> = HashSet::from([target]);
        while let Some((cur, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let next_nodes: Vec<GId> = if backward {
                self.sites_reaching(cur)
                    .into_iter()
                    .filter(|s| s.kind == SiteKind::Call && s.certain)
                    .map(|s| s.owner)
                    .collect()
            } else {
                self.sites_within(cur)
                    .into_iter()
                    .filter(|s| s.kind == SiteKind::Call && s.certain)
                    .map(|s| s.target)
                    .collect()
            };
            for next in next_nodes {
                if !seen.insert(next) {
                    continue;
                }
                parent.insert(next, cur);
                order.push(next);
                if self.exports.def(next).kind != DefKind::Module {
                    queue.push_back((next, depth + 1));
                }
            }
        }
        order
            .into_iter()
            .map(|node| {
                let mut path = vec![node];
                let mut cur = node;
                while let Some(&p) = parent.get(&cur) {
                    if p == target {
                        break;
                    }
                    path.push(p);
                    cur = p;
                }
                path.reverse();
                path
            })
            .collect()
    }
}

fn extract_one(
    parser: &mut Parser,
    bytes: &[u8],
    rel: &str,
    prefix: &[String],
    facts_path: &Path,
    stats: &mut BuildStats,
) -> Result<(Option<FactSet>, FileEntry), FlowspecError> {
    let hash = hash_file(rel, bytes);
    let (module, is_package) = module_name(prefix, rel);
    let src = String::from_utf8_lossy(bytes);
    stats.parsed += 1;
    match facts::extract(parser, &src, rel, &module, is_package) {
        Ok(f) => {
            write_atomic(facts_path, &f)?;
            let mut imports: Vec<String> = f.imports.iter().map(|i| i.module.clone()).collect();
            imports.extend(f.stars.iter().cloned());
            imports.sort();
            imports.dedup();
            let entry = FileEntry {
                rel: rel.to_string(),
                hash,
                imports,
                skipped: None,
                has_errors: f.has_errors,
                call_count: f.call_count,
            };
            Ok((Some(f), entry))
        }
        Err(reason) => {
            stats.skipped += 1;
            Ok((
                None,
                FileEntry {
                    rel: rel.to_string(),
                    hash,
                    imports: Vec::new(),
                    skipped: Some(
                        match reason {
                            facts::SkipReason::Parse => "parse",
                            facts::SkipReason::TooDeep => "too deep",
                        }
                        .to_string(),
                    ),
                    has_errors: false,
                    call_count: 0,
                },
            ))
        }
    }
}

fn list_capped(items: &[String], cap: usize) -> String {
    if items.len() <= cap {
        items.join(", ")
    } else {
        format!(
            "{}, … and {} more",
            items[..cap].join(", "),
            items.len() - cap
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(files: &[(&str, &str)]) -> (tempfile::TempDir, Index) {
        let dir = tempfile::TempDir::new().unwrap();
        let mut paths = Vec::new();
        for (rel, body) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, body).unwrap();
            paths.push(p);
        }
        let idx = Index::build(dir.path(), &paths).unwrap();
        (dir, idx)
    }

    fn callers(idx: &Index, symbol: &str) -> Vec<(String, u32, bool)> {
        let id = idx.resolve_symbol(symbol).unwrap();
        idx.sites_reaching(id)
            .into_iter()
            .filter(|s| s.kind == SiteKind::Call)
            .map(|s| (idx.exports.entity_id(s.owner), s.line, s.certain))
            .collect()
    }

    const SHEET: &str = "class Sheet:\n    def template_variables(self, n):\n        return {\"n\": n}\n\ndef make():\n    return Sheet()\n";
    const USER: &str = "from pkg.sheet import Sheet, make\n\nLIMIT = 3\n\ndef direct(sheet: Sheet):\n    return sheet.template_variables(LIMIT)\n\ndef constructed():\n    s = Sheet()\n    return s.template_variables(1)\n\ndef untyped(sheet):\n    return sheet.template_variables(2)\n\nclass Holder:\n    def __init__(self):\n        self.sheet = make()\n    def go(self):\n        return self.sheet.template_variables(4)\n";

    #[test]
    fn method_callers_cover_annotation_constructor_attribute_and_untyped() {
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            ("pkg/sheet.py", SHEET),
            ("pkg/user.py", USER),
        ]);
        assert_eq!(
            callers(&idx, "sheet.py::Sheet::template_variables"),
            vec![
                ("pkg/user.py::direct".to_string(), 6, true),
                ("pkg/user.py::constructed".to_string(), 10, true),
                ("pkg/user.py::Holder::go".to_string(), 19, true),
                ("pkg/user.py::untyped".to_string(), 13, false),
            ]
        );
        assert_eq!(
            callers(&idx, "sheet.py::make"),
            vec![("pkg/user.py::Holder::__init__".to_string(), 17, true)]
        );
    }

    #[test]
    fn script_style_sibling_and_subdirectory_imports_resolve() {
        // No package prefix: a script's own directory is on `sys.path`.
        let (_d, idx) = build(&[
            ("mod_a.py", "from mod_b import func_b\n\ndef func_a():\n    return func_b()\n"),
            ("mod_b.py", "from mod_a import func_a\n\ndef func_b():\n    return func_a()\n"),
            (
                "api/handler.py",
                "from repository.repo import load_record\n\ndef handle_request():\n    return load_record()\n",
            ),
            ("repository/repo.py", "def load_record():\n    return {\"id\": 1}\n"),
            ("deep/main.py", "from worker import Worker\n\nclass Runner:\n    def __init__(self):\n        self.x = Worker()\n    def run(self):\n        return self.x.do_job()\n"),
            ("deep/worker.py", "class Worker:\n    def do_job(self):\n        return \"done\"\n"),
        ]);
        assert_eq!(
            callers(&idx, "mod_b.py::func_b"),
            vec![("mod_a.py::func_a".to_string(), 4, true)]
        );
        assert_eq!(
            callers(&idx, "mod_a.py::func_a"),
            vec![("mod_b.py::func_b".to_string(), 4, true)]
        );
        assert_eq!(
            callers(&idx, "repo.py::load_record"),
            vec![("api/handler.py::handle_request".to_string(), 4, true)]
        );
        assert_eq!(
            callers(&idx, "worker.py::Worker::do_job"),
            vec![("deep/main.py::Runner::run".to_string(), 7, true)]
        );
    }

    #[test]
    fn aliases_super_classmethod_static_same_name_and_with_enter() {
        let shapes = "def plain():\n    return 1\n\nclass Base:\n    def ping(self):\n        return 1\n\nclass Child(Base):\n    def ping(self):\n        return super().ping()\n\nclass One:\n    def same(self):\n        return 1\n    def __enter__(self):\n        return self\n    def __exit__(self, *a):\n        pass\n\nclass Two:\n    def same(self):\n        return 2\n\nclass Modes:\n    @classmethod\n    def class_run(cls):\n        return Child().ping()\n    @staticmethod\n    def static_run():\n        return One().same()\n\ndef use(b: Base):\n    Two().same()\n    Modes.class_run()\n    Modes.static_run()\n    with One() as o:\n        o.same()\n    return b.ping()\n";
        let user = "import pkg.shapes as sh\nfrom pkg.shapes import plain as p\n\ndef through_module_alias():\n    return sh.plain()\n\ndef through_from_alias():\n    return p()\n";
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            ("pkg/shapes.py", shapes),
            ("pkg/user.py", user),
        ]);
        let owners = |sym: &str, certain: bool| -> Vec<String> {
            callers(&idx, sym)
                .into_iter()
                .filter(|c| c.2 == certain)
                .map(|c| c.0)
                .collect()
        };
        assert_eq!(
            owners("shapes.py::plain", true),
            vec![
                "pkg/user.py::through_module_alias",
                "pkg/user.py::through_from_alias"
            ]
        );
        assert_eq!(
            owners("shapes.py::Base::ping", true),
            vec!["pkg/shapes.py::Child::ping", "pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Child::ping", true),
            vec!["pkg/shapes.py::Modes::class_run"]
        );
        assert_eq!(
            owners("shapes.py::Child::ping", false),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::One::same", true),
            vec!["pkg/shapes.py::Modes::static_run", "pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Two::same", true),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Modes::class_run", true),
            vec!["pkg/shapes.py::use"]
        );
        assert_eq!(
            owners("shapes.py::Modes::static_run", true),
            vec!["pkg/shapes.py::use"]
        );
    }

    #[test]
    fn hostile_cache_is_rebuilt_not_trusted() {
        // A fact file whose indices point outside its own definitions, a
        // sites file naming definitions that do not exist, and a symlinked
        // facts directory are each refused; the index still builds from
        // source and answers correctly.
        let (dir, idx) = build(&[(
            "a.py",
            "def f():\n    return 1\n\ndef g():\n    return f()\n",
        )]);
        let files = vec![dir.path().join("a.py")];
        let facts_dir = idx.dir().join("facts");
        let fact_path = fs::read_dir(&facts_dir)
            .unwrap()
            .flatten()
            .next()
            .unwrap()
            .path();
        let mut bad: FactSet = read_bin(&fact_path).unwrap();
        bad.refs[0].owner = 999;
        write_atomic(&fact_path, &bad).unwrap();
        let sites_path = idx.dir().join("sites.bin");
        let mut sites: Vec<Site> = read_bin(&sites_path).unwrap();
        sites.push(Site {
            target: (7, 7),
            owner: (0, 1),
            line: 1,
            column: 1,
            kind: SiteKind::Call,
            certain: true,
            why: String::new(),
        });
        write_atomic(&sites_path, &sites).unwrap();
        let warm = Index::build(dir.path(), &files).unwrap();
        assert_eq!(
            callers(&warm, "a.py::f"),
            vec![("a.py::g".to_string(), 5, true)]
        );
        assert!(warm
            .all_sites()
            .iter()
            .all(|s| s.target.0 == 0 && s.target.1 < 3));

        // A cache file written by someone else carries no tag for this key:
        // it is treated as absent and the file is re-extracted.
        let raw = bincode::serde::encode_to_vec(&bad, bincode::config::standard()).unwrap();
        fs::write(&fact_path, raw).unwrap();
        let foreign = Index::build(dir.path(), &files).unwrap();
        assert_eq!(
            foreign.stats.parsed, 1,
            "untagged fact file must be re-extracted"
        );
        assert_eq!(
            callers(&foreign, "a.py::f"),
            vec![("a.py::g".to_string(), 5, true)]
        );

        let other = tempfile::TempDir::new().unwrap();
        fs::remove_dir_all(&facts_dir).unwrap();
        std::os::unix::fs::symlink(other.path(), &facts_dir).unwrap();
        let err = match Index::build(dir.path(), &files) {
            Ok(_) => panic!("a symlinked facts directory must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("symlink"), "{err}");
    }

    #[test]
    fn resolver_losses_name_annotations_dynamic_lookups_match_and_relative_imports() {
        use super::super::resolve::LossKind;
        let src = "from .nowhere import x\nfrom builtins import getattr as lookup\nimport helpers\n\ndef getattr(o, n):\n    return 1\n\nclass Known:\n    pass\n\ndef api(a: Known, b: Missing, c: int) -> Known:\n    getattr(a, 'x')\n    lookup(helpers, 'y')\n    eval('1')\n    match c:\n        case 1:\n            pass\n    return a\n";
        let (_d, idx) = build(&[("pkg/__init__.py", ""), ("pkg/m.py", src)]);
        let kinds: Vec<LossKind> = idx.losses().iter().map(|l| l.kind).collect();
        let count = |k: LossKind| kinds.iter().filter(|x| **x == k).count();
        // Four annotations (a, b, c, return): each lacks type-flow evidence.
        assert_eq!(count(LossKind::TypeFlows), 4, "{kinds:?}");
        // Only `Missing` is an unbound annotation root; `Known` and `int` are not.
        assert_eq!(count(LossKind::UnresolvedType), 1, "{kinds:?}");
        // `getattr` is shadowed by a local definition (no loss); `lookup` is
        // an alias of the builtin getattr (loss); `eval` is the builtin (loss).
        assert_eq!(count(LossKind::DynamicNamespace), 2, "{kinds:?}");
        assert_eq!(count(LossKind::MatchStatement), 1, "{kinds:?}");
        // `from .nowhere import x` names a sibling module that does not exist.
        assert_eq!(count(LossKind::UnresolvedImport), 1, "{kinds:?}");
        for l in idx.losses() {
            assert!(l.start < l.end && (l.end as usize) <= src.len(), "{l:?}");
        }
    }

    #[test]
    fn forward_slice_matches_backward_slice_and_ids_are_cached() {
        let (_d, idx) = build(&[(
            "a.py",
            "def f():\n    return 1\n\ndef g():\n    return f() + f()\n\ndef h():\n    return g()\n",
        )]);
        let f = idx.resolve_symbol("a.py::f").unwrap();
        let g = idx.resolve_symbol("a.py::g").unwrap();
        let h = idx.resolve_symbol("a.py::h").unwrap();
        let within_g: Vec<_> = idx
            .sites_within(g)
            .into_iter()
            .filter(|s| s.kind == SiteKind::Call)
            .map(|s| s.target)
            .collect();
        assert_eq!(within_g, vec![f, f]);
        assert!(idx.sites_within(h).iter().any(|s| s.target == g));
        assert!(idx.sites_within(f).is_empty());
        // Every site is reachable from both directions.
        for s in idx.all_sites() {
            assert!(idx.sites_within(s.owner).contains(s), "{s:?}");
            assert!(idx
                .sites_reaching(s.target)
                .iter()
                .any(|r| r.owner == s.owner && r.line == s.line));
        }
        let ids = idx.entity_ids();
        assert_eq!(ids.get(&g).map(String::as_str), Some("a.py::g"));
        assert!(
            std::ptr::eq(ids, idx.entity_ids()),
            "entity ids are computed once"
        );
    }

    #[test]
    fn identical_files_in_different_directories_keep_their_own_identity_when_warm() {
        // Both copies have the same bytes. A content-only fact key would hand
        // the second file the first file's facts (path, module) on a rebuild.
        let src = "def list_rules():\n    return []\n\ndef main():\n    return list_rules()\n";
        let (dir, _) = build(&[("good/oracle.py", src), ("bad/oracle.py", src)]);
        let files: Vec<PathBuf> = ["good/oracle.py", "bad/oracle.py"]
            .iter()
            .map(|p| dir.path().join(p))
            .collect();
        let warm = Index::build(dir.path(), &files).unwrap();
        for rel in ["good/oracle.py", "bad/oracle.py"] {
            let g = warm.resolve_symbol(&format!("{rel}::list_rules")).unwrap();
            assert_eq!(warm.exports.entity_id(g), format!("{rel}::list_rules"));
            assert_eq!(
                callers(&warm, &format!("{rel}::list_rules")),
                vec![(format!("{rel}::main"), 5, true)]
            );
        }
    }

    #[test]
    fn method_call_results_are_typed_by_return_annotation() {
        let src = "class Conn:\n    def cursor(self) -> \"Cursor\":\n        return Cursor()\n\nclass Cursor:\n    def execute(self, q):\n        pass\n\ndef run(c: Conn):\n    cur = c.cursor()\n    cur.execute(\"x\")\n    c.cursor().execute(\"y\")\n";
        let (_d, idx) = build(&[("db.py", src)]);
        assert_eq!(
            callers(&idx, "Cursor.execute"),
            vec![
                ("db.py::run".to_string(), 11, true),
                ("db.py::run".to_string(), 12, true)
            ]
        );
    }

    #[test]
    fn function_local_and_relative_imports_and_f_strings() {
        let a = "def helper():\n    return 1\n\ndef fmt(x):\n    return str(x)\n";
        let b = "def run():\n    from .a import helper\n    return helper()\n\ndef shadow(helper):\n    return helper()\n\ndef show(v):\n    from pkg.a import fmt\n    return f\"took {fmt(v)}\"\n";
        let (_d, idx) = build(&[("pkg/__init__.py", ""), ("pkg/a.py", a), ("pkg/b.py", b)]);
        assert_eq!(
            callers(&idx, "a.py::helper"),
            vec![("pkg/b.py::run".to_string(), 3, true)]
        );
        assert_eq!(
            callers(&idx, "a.py::fmt"),
            vec![("pkg/b.py::show".to_string(), 10, true)]
        );
    }

    #[test]
    fn rebuild_reuses_unchanged_facts_and_re_resolves_importers_only() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = dir.path().join("a.py");
        let b = dir.path().join("b.py");
        let c = dir.path().join("c.py");
        fs::write(&a, "def f():\n    return 1\n").unwrap();
        fs::write(&b, "from a import f\n\ndef g():\n    return f()\n").unwrap();
        fs::write(&c, "def h():\n    return 2\n").unwrap();
        let paths = vec![a.clone(), b.clone(), c.clone()];
        let idx = Index::build(dir.path(), &paths).unwrap();
        assert_eq!(idx.stats.parsed, 3);
        assert_eq!(idx.stats.resolved, 3);
        assert_eq!(callers(&idx, "a.py::f").len(), 1);
        // Touch a: a and b are re-resolved, c is not; nothing but a is re-parsed.
        fs::write(&a, "def f():\n    return 3\n\ndef f2():\n    return f()\n").unwrap();
        let idx = Index::build(dir.path(), &paths).unwrap();
        assert_eq!(idx.stats.parsed, 1);
        assert_eq!(idx.stats.resolved, 2);
        assert_eq!(callers(&idx, "a.py::f").len(), 2);
        assert_eq!(callers(&idx, "c.py::h").len(), 0);
    }

    #[test]
    fn an_import_in_an_outer_function_binds_names_in_inner_functions() {
        let loader = "class Loader:\n    @staticmethod\n    def load(p):\n        return 1\n";
        let state = "class Backend:\n    def load(self):\n        return 2\n";
        let user = "from pkg.state import Backend\n\nclass Manager:\n    def outer(self):\n        from pkg.loader import Loader\n        def inner():\n            return Loader.load(1)\n        return inner()\n    def other(self, b: Backend):\n        return b.load()\n";
        let (_d, idx) = build(&[
            ("pkg/__init__.py", ""),
            ("pkg/loader.py", loader),
            ("pkg/state.py", state),
            ("pkg/user.py", user),
        ]);
        assert_eq!(
            callers(&idx, "loader.py::Loader::load"),
            vec![("pkg/user.py::Manager::outer::inner".to_string(), 7, true)]
        );
        assert_eq!(
            callers(&idx, "state.py::Backend::load"),
            vec![("pkg/user.py::Manager::other".to_string(), 10, true)]
        );
    }

    #[test]
    fn module_variables_are_definitions_with_reference_sites() {
        let (_d, idx) = build(&[
            ("cfg.py", "LIMIT = 3\n_retries = 2\nhandler = helper\n\ndef helper():\n    return LIMIT + _retries\n\ndef unused():\n    return 1\n"),
            ("use.py", "from cfg import LIMIT, handler\n\ndef go():\n    return handler() + LIMIT\n"),
        ]);
        let limit = idx.resolve_symbol("cfg.py::LIMIT").unwrap();
        assert_eq!(idx.exports.def(limit).kind, DefKind::Variable);
        let refs: Vec<(String, u32)> = idx
            .sites_reaching(limit)
            .into_iter()
            .filter(|s| s.kind == SiteKind::Reference && s.certain)
            .map(|s| (idx.exports.entity_id(s.owner), s.line))
            .collect();
        assert_eq!(
            refs,
            vec![
                ("cfg.py::helper".to_string(), 6),
                ("use.py::go".to_string(), 4)
            ]
        );
        // `handler = helper` then `handler()`: the call reaches `helper` through the alias.
        assert_eq!(
            callers(&idx, "cfg.py::helper"),
            vec![("use.py::go".to_string(), 4, true)]
        );
    }

    #[test]
    fn symbol_spellings_resolve_and_errors_list_the_file() {
        let (_d, idx) = build(&[("pkg/__init__.py", ""), ("pkg/sheet.py", SHEET)]);
        let want = idx
            .resolve_symbol("pkg/sheet.py::Sheet::template_variables")
            .unwrap();
        for q in [
            "sheet.py::Sheet::template_variables",
            "sheet.py::template_variables",
            "Sheet.template_variables",
            "Sheet::template_variables",
            "template_variables",
        ] {
            assert_eq!(idx.resolve_symbol(q).unwrap(), want, "{q}");
        }
        let err = idx
            .resolve_symbol("sheet.py::nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("Available in this file"), "{err}");
    }

    #[test]
    fn transitive_callers_are_one_shortest_path_each() {
        let src = "def leaf():\n    return 1\n\ndef mid():\n    return leaf()\n\ndef top():\n    return mid()\n\ndef also():\n    return mid() + leaf()\n";
        let (_d, idx) = build(&[("m.py", src)]);
        let leaf = idx.resolve_symbol("leaf").unwrap();
        let paths: Vec<Vec<String>> = idx
            .transitive_callers(leaf, 10)
            .into_iter()
            .map(|p| {
                p.into_iter()
                    .map(|g| idx.exports.def(g).name.clone())
                    .collect()
            })
            .collect();
        assert_eq!(paths, vec![vec!["mid"], vec!["also"], vec!["mid", "top"]]);
    }

    #[test]
    fn star_import_fans_terminate_and_deep_files_are_skipped() {
        let mut files: Vec<(String, String)> = vec![("pkg/__init__.py".into(), String::new())];
        for n in 0..12 {
            let body: String = (0..12)
                .filter(|m| *m != n)
                .map(|m| format!("from pkg.m{m} import *\n"))
                .collect();
            files.push((format!("pkg/m{n}.py"), body));
        }
        files.push((
            "pkg/use.py".into(),
            "from pkg.m0 import missing\n\ndef f():\n    return missing()\n".into(),
        ));
        let deep = format!("x = {}1{}\n", "(".repeat(400), ")".repeat(400));
        files.push(("pkg/deep.py".into(), deep));
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let started = std::time::Instant::now();
        let (_d, idx) = build(&refs);
        assert!(started.elapsed().as_secs() < 5);
        assert_eq!(idx.stats.skipped, 1);
        assert!(idx.resolve_symbol("use.py::f").is_ok());
    }
}
