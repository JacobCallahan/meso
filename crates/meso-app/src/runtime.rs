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
    let (sender, receiver) = std::sync::mpsc::channel::<T>();

    runtime().spawn(async move {
        let result = task.await;
        let _ = sender.send(result);
    });

    // `on_complete` is FnOnce so wrap it in Option to satisfy the Fn bound
    // required by glib::timeout_add_local.
    let mut on_complete = Some(on_complete);

    // Poll the channel on the GLib main loop
    glib::timeout_add_local(
        std::time::Duration::from_millis(10),
        move || match receiver.try_recv() {
            Ok(result) => {
                if let Some(cb) = on_complete.take() {
                    cb(result);
                }
                glib::ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
        },
    );
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
