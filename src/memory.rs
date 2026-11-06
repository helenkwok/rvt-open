//! Available-memory check.
//!
//! The reader has no fixed size cap on legitimate data: a multi-GB model is
//! read in full, as Revit reads it. What stops a hostile or oversized file
//! instead is this check, made before a large allocation: when the memory
//! the process may still use is smaller than what the next step needs, the
//! read fails with an error that says so, rather than being killed by the
//! operating system or cutting the data short.
//!
//! On Linux the figure is the smaller of `MemAvailable` (`/proc/meminfo`) and
//! the cgroup's remaining budget (`memory.max` minus `memory.current`, cgroup
//! v2, or the v1 equivalents), so a capped container is respected. Elsewhere
//! it is unknown and the check passes.

/// Bytes of memory the process can still allocate, when the platform says.
pub fn available_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let read = |p: &str| std::fs::read_to_string(p).ok();
        let v2 = read("/sys/fs/cgroup/memory.max").is_some();
        let (max, current, stat) = if v2 {
            (
                read("/sys/fs/cgroup/memory.max"),
                read("/sys/fs/cgroup/memory.current"),
                read("/sys/fs/cgroup/memory.stat"),
            )
        } else {
            (
                read("/sys/fs/cgroup/memory/memory.limit_in_bytes"),
                read("/sys/fs/cgroup/memory/memory.usage_in_bytes"),
                read("/sys/fs/cgroup/memory/memory.stat"),
            )
        };
        available_from(
            read("/proc/meminfo").as_deref(),
            max.as_deref(),
            current.as_deref(),
            stat.as_deref(),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// [`available_bytes`] from the text of `/proc/meminfo` and the cgroup's
/// limit, usage and `memory.stat` (v2 or v1), each `None` when absent.
///
/// The cgroup's usage counts page cache, which the kernel reclaims before
/// it fails an allocation: a container that has just read a few GB of
/// models shows almost no headroom while most of it is cache. So the
/// cgroup figure is limit - usage + the file pages in `memory.stat`
/// (`active_file` + `inactive_file`, or v1's `total_` forms). No limit
/// (`max`, or v1's huge sentinel) means only `MemAvailable` applies, which
/// already counts reclaimable cache. The smaller of the two is returned.
pub fn available_from(
    meminfo: Option<&str>,
    cgroup_max: Option<&str>,
    cgroup_current: Option<&str>,
    cgroup_stat: Option<&str>,
) -> Option<u64> {
    let host = meminfo.and_then(|s| {
        s.lines()
            .find_map(|l| l.strip_prefix("MemAvailable:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
            .map(|kb| kb * 1024)
    });
    let num = |s: Option<&str>| s.and_then(|s| s.trim().parse::<u64>().ok());
    let stat = |key: &str| {
        cgroup_stat.and_then(|s| {
            s.lines().find_map(|l| {
                let (k, v) = l.split_once(' ')?;
                (k == key).then(|| v.trim().parse::<u64>().ok()).flatten()
            })
        })
    };
    let cgroup = match (num(cgroup_max), num(cgroup_current)) {
        (Some(max), Some(cur)) if max < (1 << 60) => {
            let file = stat("active_file")
                .or(stat("total_active_file"))
                .unwrap_or(0)
                + stat("inactive_file")
                    .or(stat("total_inactive_file"))
                    .unwrap_or(0);
            Some(max.saturating_sub(cur).saturating_add(file).min(max))
        }
        _ => None,
    };
    match (host, cgroup) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// `Err` naming `what` when `needed` bytes exceed the memory available.
pub fn ensure_available(needed: u64, what: &str) -> crate::Result<()> {
    match available_bytes() {
        Some(available) if needed > available => Err(crate::Error::InsufficientMemory(format!(
            "{what} needs {} MiB but only {} MiB is available; stopped before allocating",
            needed >> 20,
            available >> 20
        ))),
        _ => Ok(()),
    }
}
