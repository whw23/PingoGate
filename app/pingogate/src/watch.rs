//! Optional file-watch reload trigger (FR-019).
//!
//! A lightweight modified-time poller — no inotify dependency, so it behaves
//! predictably on WSL and network mounts. When the config file's mtime changes,
//! it reloads through the shared [`Reloader`]. Disabled unless the binary is
//! told to enable it (see `PINGO_WATCH_INTERVAL_SECS`), keeping the default
//! deployment trigger explicit (SIGHUP / Admin API).

use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use pingo_admin::Reloader;

/// Spawn a background thread that reloads when `path`'s mtime changes.
pub fn spawn_file_watcher(reloader: Arc<Reloader>, path: PathBuf, interval: Duration) {
    let spawned = thread::Builder::new()
        .name("pingogate-watch".to_string())
        .spawn(move || run(reloader, path, interval));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "failed to spawn config watcher; file-watch reload disabled");
    }
}

fn run(reloader: Arc<Reloader>, path: PathBuf, interval: Duration) {
    let mut last = mtime(&path);
    tracing::info!(path = %path.display(), "config file watcher installed");
    loop {
        thread::sleep(interval);
        let current = mtime(&path);
        // Only react to a real, observable change to an existing file; a
        // transiently-missing file (mid-write) is ignored until it reappears.
        if current.is_some() && current != last {
            last = current;
            reloader.trigger("file-watch");
        }
    }
}

fn mtime(path: &PathBuf) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
