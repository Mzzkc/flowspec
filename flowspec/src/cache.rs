//! Content-addressed structured per-file IR storage. Global graphs are never cached here.
//! Linux directory handles pin the cache namespace against symlink replacement.
use crate::parser::ir::{Boundary, ParseResult, Reference, Scope, Symbol};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::{Component, Path},
    sync::atomic::{AtomicU64, Ordering},
};
const MAX_BLOB: u64 = 8 * 1024 * 1024;
const O_NOFOLLOW: i32 = 0o400000;
const O_NONBLOCK: i32 = 0o4000;
const MAGIC: &[u8] = b"FSIR0001";
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalysisMode {
    #[default]
    Full,
    Incremental,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub parsed: u64,
    pub fallbacks: Vec<String>,
}
/// The report uses the adapter owner's public serialized schema, not a second taxonomy.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub ir: ParseResult,
    pub report: Value,
}
#[derive(Serialize, Deserialize)]
struct SerializableOutput {
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    scopes: Vec<Scope>,
    boundaries: Vec<Boundary>,
    report: Value,
}
impl From<ParsedFile> for SerializableOutput {
    fn from(p: ParsedFile) -> Self {
        Self {
            symbols: p.ir.symbols,
            references: p.ir.references,
            scopes: p.ir.scopes,
            boundaries: p.ir.boundaries,
            report: p.report,
        }
    }
}
impl From<SerializableOutput> for ParsedFile {
    fn from(p: SerializableOutput) -> Self {
        Self {
            ir: ParseResult {
                symbols: p.symbols,
                references: p.references,
                scopes: p.scopes,
                boundaries: p.boundaries,
            },
            report: p.report,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CacheKey {
    pub digest: String,
    pub source_sha256: String,
    pub source_bytes: u64,
}
impl CacheKey {
    /// Caller supplies every effective policy, scope, adapter, registry and parser input.
    pub fn new(identity: &Value, source: &[u8]) -> Self {
        let source_sha256 = format!("{:x}", Sha256::digest(source));
        let bound = json!({"cache_schema":1,"report_abi":"v1","identity":identity,"source_sha256":source_sha256,"source_bytes":source.len()});
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&bound).expect("JSON cache identity"))
        );
        Self {
            digest,
            source_sha256,
            source_bytes: source.len() as u64,
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Record {
    key: CacheKey,
    output: SerializableOutput,
}
pub(crate) fn valid_output(output: &ParsedFile, key: &CacheKey) -> bool {
    let report = &output.report;
    report["schema_version"] == 1
        && report["source_sha256"] == key.source_sha256
        && report["source_bytes"] == key.source_bytes
        && report["grammar_version"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && !s.contains("legacy") && !s.contains("unknown"))
        && report["adapter_version"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
        && report["capabilities"].is_object()
        && report["events"]
            .as_array()
            .is_some_and(|events| !events.iter().any(|e| e["code"] == "unknown_legacy_report"))
}
fn error(reason: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, reason)
}
fn open_directory(path: &Path) -> std::io::Result<File> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut directory = File::open("/")?;
    for part in absolute.components() {
        let name = match part {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(name) => name,
            _ => return Err(error("cache path traversal refused")),
        };
        let next = Path::new("/proc/self/fd")
            .join(directory.as_raw_fd().to_string())
            .join(name);
        match fs::create_dir(&next) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(next)?;
        if !opened.metadata()?.is_dir() {
            return Err(error("cache component is not a directory"));
        }
        directory = opened;
    }
    Ok(directory)
}
pub struct CacheStore {
    directory: File,
}
impl CacheStore {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        Ok(Self {
            directory: open_directory(path)?,
        })
    }
    fn path(&self, name: &str) -> std::path::PathBuf {
        Path::new("/proc/self/fd")
            .join(self.directory.as_raw_fd().to_string())
            .join(name)
    }
    fn name(key: &CacheKey) -> std::io::Result<String> {
        if key.digest.len() != 64 || !key.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(error("invalid cache key"));
        }
        Ok(format!("{}.ir", key.digest))
    }
    pub fn load(&self, key: &CacheKey) -> std::io::Result<Option<ParsedFile>> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(self.path(&Self::name(key)?))
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.len() > MAX_BLOB {
            return Err(error("cache blob exceeds limit or is not regular"));
        }
        let mut data = Vec::new();
        file.take(MAX_BLOB + 1).read_to_end(&mut data)?;
        if data.len() as u64 > MAX_BLOB || data.len() < 72 || &data[..8] != MAGIC {
            return Err(error("invalid cache framing"));
        }
        let payload = &data[72..];
        let checksum = format!("{:x}", Sha256::digest(payload));
        if checksum.as_bytes() != &data[8..72] {
            return Err(error("cache checksum mismatch"));
        }
        let record: Record =
            serde_json::from_slice(payload).map_err(|_| error("invalid cache payload"))?;
        if record.key != *key {
            return Err(error("cache identity mismatch"));
        }
        let output = ParsedFile::from(record.output);
        if !valid_output(&output, key) {
            return Err(error("unverified or incompatible structured report"));
        }
        Ok(Some(output))
    }
    pub fn store(&self, key: &CacheKey, output: ParsedFile) -> std::io::Result<()> {
        if !valid_output(&output, key) {
            return Err(error("legacy or unbound parse output cannot be cached"));
        }
        let final_path = self.path(&Self::name(key)?);
        let mut payload = BoundedBytes(Vec::new());
        serde_json::to_writer(
            &mut payload,
            &Record {
                key: key.clone(),
                output: output.into(),
            },
        )
        .map_err(|_| error("cannot serialize bounded cache output"))?;
        let payload = payload.0;
        if payload.len() as u64 + 72 > MAX_BLOB {
            return Err(error("cache output exceeds limit"));
        }
        let lock_path = self.path(".writer.lock");
        let _lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(O_NOFOLLOW | O_NONBLOCK)
            .open(&lock_path)?;
        let temp = self.path(&format!(
            ".{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        let mut owns_temp = false;
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(O_NOFOLLOW | O_NONBLOCK)
                .open(&temp)?;
            owns_temp = true;
            file.write_all(MAGIC)?;
            file.write_all(format!("{:x}", Sha256::digest(&payload)).as_bytes())?;
            file.write_all(&payload)?;
            file.sync_all()?;
            fs::rename(&temp, &final_path)?;
            self.directory.sync_all()
        })();
        if owns_temp {
            let _ = fs::remove_file(&temp);
        }
        let _ = fs::remove_file(lock_path);
        result
    }
}

/// Reduce serialized structured events and capability debt into the existing report normal form.
pub(crate) fn reduce_parse_report(
    report: &Value,
    root: &Path,
    file: &Path,
    source: &str,
    registry: &crate::registry::Registry,
    losses: &mut Vec<Value>,
) {
    let path = crate::report_authority::path_key(root, file);
    for event in report["events"].as_array().into_iter().flatten() {
        let code = match event["code"].as_str().unwrap_or("unknown_event") {
            "parser_recovery" | "missing_node" => "parse_recovery",
            "depth_truncated" => "depth_truncation",
            other => other,
        };
        let range = event["byte_start"]
            .as_u64()
            .zip(event["byte_end"].as_u64())
            .map(|(a, b)| crate::report_authority::offset_range(&path, source, a, b));
        let mut loss = crate::report_authority::loss(code, path.clone(), range, "parse");
        let affects: Vec<&str> = event["affects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let patterns: Vec<_> = registry
            .patterns()
            .iter()
            .filter(|p| p.required_capabilities.iter().any(|c| affects.contains(c)))
            .map(|p| p.id)
            .collect();
        loss["required_capabilities"] = json!(affects);
        loss["affected_patterns"] = json!(patterns);
        losses.push(loss);
    }
    for pattern in registry.patterns() {
        let missing: Vec<_> = pattern
            .required_capabilities
            .iter()
            .filter(|cap| report["capabilities"][**cap] != "implemented")
            .copied()
            .collect();
        if report["schema_version"] != 1 || !missing.is_empty() {
            let mut loss = crate::report_authority::loss(
                "capability_unavailable",
                path.clone(),
                None,
                "adapter_capability",
            );
            loss["required_capabilities"] = json!(missing);
            loss["affected_patterns"] = json!([pattern.id]);
            losses.push(loss);
        }
    }
}

struct BoundedBytes(Vec<u8>);
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) as u64 > MAX_BLOB - 72 {
            return Err(error("cache output exceeds limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
