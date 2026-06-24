//! Background scanning.
//!
//! The scan runs on its own thread so the UI never blocks. Results come back
//! over a channel; the UI polls it each frame and transitions the [`Scan`] state
//! machine from `Running` to `Done` (or `Error`) when the result arrives.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use scanner::Tree;

/// The current state of the one scan the app cares about.
pub enum Scan {
    Running {
        started: Instant,
        rx: Receiver<Result<Tree, String>>,
    },
    Done {
        tree: Tree,
        elapsed: Duration,
    },
    Error(String),
}

impl Scan {
    /// Spawn a scan of `path` on a background thread and return immediately.
    pub fn start(path: &Path) -> Scan {
        let (tx, rx) = mpsc::channel();
        let path = path.to_path_buf();
        thread::spawn(move || {
            // Whole physical filesystem, crossing btrfs subvolumes / same-device
            // mounts (so picking a device shows everything on it).
            let result = scanner::scan_filesystem(&path).map_err(|e| e.to_string());
            // The receiver may already be gone (e.g. a rescan superseded us);
            // dropping the result then is fine.
            let _ = tx.send(result);
        });
        Scan::Running {
            started: Instant::now(),
            rx,
        }
    }

    /// Non-blocking poll. Transitions `Running` -> `Done`/`Error` once the scan
    /// thread reports back.
    pub fn poll(&mut self) {
        if let Scan::Running { started, rx } = self {
            match rx.try_recv() {
                Ok(Ok(tree)) => {
                    let elapsed = started.elapsed();
                    *self = Scan::Done { tree, elapsed };
                }
                Ok(Err(error)) => *self = Scan::Error(error),
                Err(TryRecvError::Empty) => {} // still scanning
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
