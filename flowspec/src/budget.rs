//! Process-wide memory budget.
//!
//! Flowspec must never take its host down. Every allocation goes through
//! [`BudgetAlloc`], which counts bytes currently held; analysis stages call
//! [`check`] at their boundaries and stop with
//! [`FlowspecError::MemoryBudgetExceeded`] once the budget is crossed. The
//! check is at stage boundaries rather than inside the allocator so the
//! failure is an ordinary error with a stage name and a non-zero exit, not an
//! abort in the middle of a data structure.
//!
//! The default budget is [`DEFAULT_BUDGET_BYTES`]; `--memory-limit` or
//! `FLOWSPEC_MEMORY_LIMIT` override it, and `0` disables the check (the
//! counter still runs so `peak()` stays accurate).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::error::FlowspecError;

/// Budget applied when nothing else is configured: 2 GiB.
pub const DEFAULT_BUDGET_BYTES: u64 = 2 * 1024 * 1024 * 1024;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static BUDGET: AtomicU64 = AtomicU64::new(DEFAULT_BUDGET_BYTES);

/// Global allocator that keeps a running count of bytes held.
///
/// Install it in a binary with `#[global_allocator] static A: BudgetAlloc = BudgetAlloc;`.
pub struct BudgetAlloc;

// SAFETY: every method forwards to `System` unchanged; the counters are
// atomics touched with relaxed ordering and never affect the returned
// pointer or layout.
unsafe impl GlobalAlloc for BudgetAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            record_alloc(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            record_alloc(layout.size());
        }
        p
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = System.realloc(ptr, layout, new_size);
        if !p.is_null() {
            CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
            record_alloc(new_size);
        }
        p
    }
}

fn record_alloc(size: usize) {
    let now = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

/// Bytes currently held through the counting allocator.
pub fn current() -> u64 {
    CURRENT.load(Ordering::Relaxed) as u64
}

/// Largest number of bytes held at any point so far.
pub fn peak() -> u64 {
    PEAK.load(Ordering::Relaxed) as u64
}

/// Set the budget in bytes; `0` disables the check.
pub fn set_budget(bytes: u64) {
    BUDGET.store(bytes, Ordering::Relaxed);
}

/// The budget in effect, in bytes (`0` means unlimited).
pub fn budget() -> u64 {
    BUDGET.load(Ordering::Relaxed)
}

/// Fail if the bytes currently held exceed the budget.
///
/// `stage` names the analysis stage that just finished, so the error says
/// where the process stood when it stopped.
pub fn check(stage: &str) -> Result<(), FlowspecError> {
    let limit = budget();
    if limit == 0 {
        return Ok(());
    }
    let held = current();
    if held > limit {
        return Err(FlowspecError::MemoryBudgetExceeded {
            stage: stage.to_string(),
            held,
            limit,
        });
    }
    Ok(())
}

/// Parse a human-readable size such as `512M`, `2G`, `1.5GiB` or `1073741824`.
pub fn parse_size(text: &str) -> Result<u64, String> {
    let t = text.trim();
    let lower = t.to_ascii_lowercase();
    let (num, mult): (&str, u64) = if let Some(n) = lower
        .strip_suffix("gib")
        .or(lower.strip_suffix("gb"))
        .or(lower.strip_suffix('g'))
    {
        (n, 1 << 30)
    } else if let Some(n) = lower
        .strip_suffix("mib")
        .or(lower.strip_suffix("mb"))
        .or(lower.strip_suffix('m'))
    {
        (n, 1 << 20)
    } else if let Some(n) = lower
        .strip_suffix("kib")
        .or(lower.strip_suffix("kb"))
        .or(lower.strip_suffix('k'))
    {
        (n, 1 << 10)
    } else if let Some(n) = lower.strip_suffix('b') {
        (n, 1)
    } else {
        (lower.as_str(), 1)
    };
    let num = num.trim();
    let value: f64 = num
        .parse()
        .map_err(|_| format!("'{text}' is not a size (examples: 512M, 2G, 0 to disable)"))?;
    if value < 0.0 || !value.is_finite() {
        return Err(format!(
            "'{text}' is not a size (examples: 512M, 2G, 0 to disable)"
        ));
    }
    Ok((value * mult as f64) as u64)
}

/// Format bytes as a short human-readable size.
pub fn format_size(bytes: u64) -> String {
    const G: f64 = (1u64 << 30) as f64;
    const M: f64 = (1u64 << 20) as f64;
    let b = bytes as f64;
    if b >= G {
        format!("{:.2} GiB", b / G)
    } else if b >= M {
        format!("{:.1} MiB", b / M)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse_in_common_spellings() {
        assert_eq!(parse_size("2G").unwrap(), 2 << 30);
        assert_eq!(parse_size("2GiB").unwrap(), 2 << 30);
        assert_eq!(parse_size("512m").unwrap(), 512 << 20);
        assert_eq!(parse_size("1.5G").unwrap(), 3 << 29);
        assert_eq!(parse_size("1024").unwrap(), 1024);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("lots").is_err());
        assert!(parse_size("-1G").is_err());
    }

    /// The library's own test binary does not install `BudgetAlloc`, so the
    /// counter is driven by hand here; the CLI tests exercise the real path.
    #[test]
    fn check_fails_only_above_budget_and_names_the_stage() {
        let saved = budget();
        set_budget(1);
        record_alloc(4096);
        let err = check("parse").unwrap_err().to_string();
        assert!(err.contains("after parse"), "{err}");
        assert!(err.contains("memory budget exceeded"), "{err}");
        assert!(err.contains("--memory-limit"), "{err}");
        set_budget(0);
        assert!(check("parse").is_ok());
        CURRENT.fetch_sub(4096, Ordering::Relaxed);
        set_budget(saved);
    }

    #[test]
    fn format_size_is_readable() {
        assert_eq!(format_size(3 << 30), "3.00 GiB");
        assert_eq!(format_size(5 << 20), "5.0 MiB");
        assert_eq!(format_size(12), "12 B");
    }
}
