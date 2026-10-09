/*
 * Tokio async runtime bridge for GTK4.
 *
 * GTK4 runs on the main thread with its own GLib event loop.
 * We spin a dedicated Tokio multi-thread runtime in a background thread,
 * and bridge results back to the GTK main thread via `glib::MainContext::spawn`.
 *
 * Usage:
 *   AsyncRuntime::spawn(async move { ... }, callback)
 *   where callback: Fn(T) + 'static is called on the GTK main thread.
 */

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

/// Get (or lazily init) the background Tokio runtime.
fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .thread_name("meso-async")
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

/// Spawn an async task and deliver its result to the GTK main thread.
///
/// `task` is executed on the Tokio runtime.
/// `on_complete` is called exactly once on the GTK main thread with the result.
pub fn spawn<F, T, C>(task: F, on_complete: C)
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
    C: FnOnce(T) + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel::<T>();

    runtime().spawn(async move {
        let result = task.await;
        let _ = tx.send(result);
    });

    glib::spawn_future_local(async move {
        if let Ok(result) = rx.await {
            on_complete(result);
        }
    });
}

/// Shared progress slot — write from async, read from GTK thread.
pub type ProgressSlot = Arc<Mutex<Option<String>>>;

/// Start a 50 ms GLib polling timer that reads from `progress` and calls `label.set_text(...)`.
/// Returns an `Rc<Cell<bool>>` — set it to `true` to stop the polling timer.
pub fn progress_poller(progress: ProgressSlot, label: gtk4::Label) -> Rc<Cell<bool>> {
    let done = Rc::new(Cell::new(false));
    let done_c = Rc::clone(&done);
    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        if done_c.get() {
            return glib::ControlFlow::Break;
        }
        if let Ok(mut g) = progress.try_lock() {
            if let Some(msg) = g.take() {
                label.set_text(&msg);
            }
        }
        glib::ControlFlow::Continue
    });
    done
}
