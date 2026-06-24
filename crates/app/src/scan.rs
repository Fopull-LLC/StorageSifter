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

use scanner::Tree;

/// The current state of the one scan the app cares about.
pub enum Scan {
    Running {
        started: Instant,
        rx: Receiver<Result<Tree, String>>,
        cancel: Arc<AtomicBool>,
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
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let path = path.to_path_buf();
        let thread_cancel = cancel.clone();
        thread::spawn(move || {
            // Whole physical filesystem, crossing btrfs subvolumes; abandons the
            // walk if the UI sets the cancel flag.
            let result = scanner::scan_filesystem_cancellable(&path, &thread_cancel)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
        Scan::Running {
            started: Instant::now(),
            rx,
            cancel,
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
