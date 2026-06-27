//! Background scanning.
//!
//! The scan runs on its own thread so the UI never blocks. Results come back
//! over a channel; the UI polls it each frame. A shared cancel flag lets the UI
//! abandon a long or superseded scan — the scanner checks it and winds down
//! promptly instead of running the whole filesystem walk to completion.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use scanner::{Progress, Tree};

/// The current state of the one scan the app cares about.
pub enum Scan {
    Running {
        started: Instant,
        rx: Receiver<Result<Tree, String>>,
        cancel: Arc<AtomicBool>,
        /// Live counts (entries + bytes), updated by the scanner thread.
        progress: Arc<Progress>,
        /// On-disk bytes the scan is expected to find (the filesystem's used
        /// space), for a percentage + ETA. `None` when unknown (e.g. a path-arg
        /// scan), in which case the UI shows an indeterminate bar.
        target: Option<u64>,
    },
    Done {
        tree: Tree,
        elapsed: Duration,
    },
    Error(String),
}

impl Scan {
    /// Spawn a whole-filesystem scan of `path` on a background thread.
    pub fn start(path: &Path) -> Scan {
        Scan::start_with_target(path, None)
    }

    /// Like [`start`], but records the expected total used bytes so the UI can
    /// show a real percentage and ETA.
    pub fn start_with_target(path: &Path, target: Option<u64>) -> Scan {
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let path = path.to_path_buf();
        let thread_cancel = cancel.clone();
        let thread_progress = progress.clone();
        thread::spawn(move || {
            // Whole physical filesystem, crossing btrfs subvolumes; abandons the
            // walk if the UI sets the cancel flag, and reports progress as it goes.
            let result = scanner::scan_filesystem_progress(&path, &thread_cancel, &thread_progress)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        Scan::Running {
            started: Instant::now(),
            rx,
            cancel,
            progress,
            target,
        }
    }

    /// Signal an in-progress scan to stop as soon as possible. The thread winds
    /// down and its (partial) result is dropped by the caller.
    pub fn cancel(&self) {
        if let Scan::Running { cancel, .. } = self {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Non-blocking poll; transitions `Running` -> `Done`/`Error` when ready.
    pub fn poll(&mut self) {
        if let Scan::Running { started, rx, .. } = self {
            match rx.try_recv() {
                Ok(Ok(tree)) => {
                    let elapsed = started.elapsed();
                    *self = Scan::Done { tree, elapsed };
                }
                Ok(Err(error)) => *self = Scan::Error(error),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    *self = Scan::Error("scan thread terminated unexpectedly".to_owned());
                }
            }
        }
    }

    pub fn is_running(&self) -> bool {
        matches!(self, Scan::Running { .. })
    }
}
