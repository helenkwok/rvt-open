//! Cooperative cancellation and progress reporting for long scans.
//!
//! Project files run the walker over tens of megabytes of `Global/Latest`
//! and every `Partitions/*` stream; a caller driving a UI or a batch job
//! needs to stop that early and to see that it is still moving. Both hooks
//! are opt-in and additive: the `*_with_limits` entry points keep their
//! signatures and behave as before, and the `*_with_control` variants take
//! a [`WalkerControl`] carrying an optional [`CancelToken`] and an optional
//! progress callback.
//!
//! Cancellation is cooperative — checked at the start of every
//! [`PROGRESS_BYTE_INTERVAL`] of scanned bytes, every 256 decoded
//! candidates, before each partition stream, and once after each loop —
//! and it returns [`Error::Cancelled`]; nothing partial is yielded. Saved
//! graphics (`research::native_saved_scene::extract_at_detail_with_control`)
//! check it before each pass over the file and before each element's mesh.

use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::error::{Error, Result};

/// Bytes of input between cancellation checks / progress reports in the
/// candidate scan (1 MiB). Power of two so the check is a mask.
pub const PROGRESS_BYTE_INTERVAL: usize = 1 << 20;

/// Shared cancellation flag. Clone it into a UI thread or a signal handler
/// and call [`CancelToken::cancel`]; the scan observes it at its next
/// checkpoint. Once cancelled it stays cancelled.
#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation. Idempotent.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// `Err(Error::Cancelled)` once [`cancel`](Self::cancel) has been called.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Which phase of a scan a [`ProgressEvent`] describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// `Formats/Latest` decompress + schema parse (reported as 0 then 1/1).
    SchemaParse,
    /// Class-tag scan over `Global/Latest`; `done`/`total` are bytes.
    CandidateScan,
    /// Typed / generic decode of scan candidates; `done`/`total` are candidates.
    ElementDecode,
    /// Partition-stream passes; `done`/`total` are streams where known,
    /// otherwise 0 then 1/1 around the merged partition recoveries.
    PartitionScan,
    /// Reading the saved display graphics and their materials (reported as
    /// 0 then 1/1).
    SavedGraphics,
    /// Meshing each element's saved graphics; `done`/`total` are elements.
    SavedMeshes,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SchemaParse => "schema-parse",
            Self::CandidateScan => "candidate-scan",
            Self::ElementDecode => "element-decode",
            Self::PartitionScan => "partition-scan",
            Self::SavedGraphics => "saved-graphics",
            Self::SavedMeshes => "saved-meshes",
        }
    }
}

/// One progress report. `total` is `None` when the stage has no known
/// denominator yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProgressEvent {
    pub stage: Stage,
    pub done: u64,
    pub total: Option<u64>,
}

/// Progress callback. Called from the scanning thread; keep it cheap.
pub type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Optional cancellation token + optional progress callback handed to the
/// `*_with_control` walker entry points. The default is a no-op that can
/// never cancel, which is what the `*_with_limits` variants use.
#[derive(Clone, Default)]
pub struct WalkerControl {
    cancel: Option<CancelToken>,
    progress: Option<ProgressCallback>,
}

impl fmt::Debug for WalkerControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalkerControl")
            .field("cancel", &self.cancel)
            .field("progress", &self.progress.as_ref().map(|_| "callback"))
            .finish()
    }
}

impl WalkerControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_cancel(mut self, token: CancelToken) -> Self {
        self.cancel = Some(token);
        self
    }

    pub fn with_progress<F>(mut self, callback: F) -> Self
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        self.progress = Some(Arc::new(callback));
        self
    }

    /// `Err(Error::Cancelled)` if a token is attached and cancelled.
    pub fn check(&self) -> Result<()> {
        match &self.cancel {
            Some(token) => token.check(),
            None => Ok(()),
        }
    }

    /// Emit a progress event if a callback is attached.
    pub fn report(&self, stage: Stage, done: u64, total: Option<u64>) {
        if let Some(progress) = &self.progress {
            progress(ProgressEvent { stage, done, total });
        }
    }
}

/// What [`set_stage_hook`] takes: called with each phase's label.
pub type StageHook = Box<dyn Fn(&str)>;

thread_local! {
    static STAGE_HOOK: RefCell<Option<StageHook>> = const { RefCell::new(None) };
}

/// Set (or with `None` clear) this thread's stage hook: [`stage`] calls it
/// with a short label as a conversion moves from one phase to the next
/// (walking elements, saved graphics, plan maps...). The browser build
/// forwards it to its progress callback and reads its memory there.
/// No hook, no cost beyond a thread-local read.
pub fn set_stage_hook(hook: Option<StageHook>) {
    STAGE_HOOK.with(|h| *h.borrow_mut() = hook);
}

/// Report that the conversion on this thread enters phase `label`.
pub fn stage(label: &str) {
    STAGE_HOOK.with(|h| {
        if let Some(f) = h.borrow().as_ref() {
            f(label);
        }
    });
}

/// How many threads [`map_in_order`] uses: one in the browser build, else
/// the machine's available parallelism.
pub(crate) fn workers() -> usize {
    if cfg!(all(target_family = "wasm", target_os = "unknown")) {
        1
    } else {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    }
}

/// `items.iter().map(f).collect()`, with the items shared out among
/// [`workers`] threads as each finishes its last. The results come back in
/// the items' order, so the output does not depend on the thread count.
pub(crate) fn map_in_order<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let workers = workers().min(items.len());
    if workers <= 1 {
        return items.iter().map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.push((i, f(item)));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    });
    done.sort_unstable_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

#[cfg(test)]
mod map_in_order_tests {
    #[test]
    fn keeps_the_order() {
        let items: Vec<u64> = (0..10_000).collect();
        let got = super::map_in_order(&items, |&x| x * x);
        assert_eq!(got, items.iter().map(|&x| x * x).collect::<Vec<_>>());
        assert!(super::map_in_order(&[] as &[u8], |&x| x).is_empty());
    }
}
