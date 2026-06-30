//! `SIGHUP` reload trigger (FR-019).
//!
//! Pingora owns the main runtime, so the signal handler runs on its own thread
//! with a dedicated current-thread Tokio runtime. Each `SIGHUP` rebuilds the
//! snapshot through the shared [`Reloader`]; a rejected candidate leaves the
//! active config serving, with the reason recorded in the reload status (XII).

use std::sync::Arc;
use std::thread;

use pingo_admin::Reloader;
use tokio::runtime::Builder;
use tokio::signal::unix::{signal, SignalKind};

/// Spawn a background thread that reloads the configuration on every `SIGHUP`.
/// A failure to install the handler is logged and disables signal-based reload
/// without affecting the data path.
pub fn spawn_sighup_handler(reloader: Arc<Reloader>) {
    let spawned = thread::Builder::new()
        .name("pingogate-sighup".to_string())
        .spawn(move || run(reloader));
    if let Err(e) = spawned {
        tracing::error!(error = %e, "failed to spawn SIGHUP thread; signal reload disabled");
    }
}

fn run(reloader: Arc<Reloader>) {
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "failed to build SIGHUP runtime; signal reload disabled");
            return;
        }
    };
    runtime.block_on(async move {
        let mut hangup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "failed to register SIGHUP handler");
                return;
            }
        };
        tracing::info!("SIGHUP reload handler installed");
        while hangup.recv().await.is_some() {
            reloader.trigger("sighup");
        }
    });
}
