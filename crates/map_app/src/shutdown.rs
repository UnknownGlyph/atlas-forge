//! The native window and graphics resources can close while file and export
//! workers finish. The main process keeps those jobs alive until they return.
use std::cell::RefCell;

type Task = Box<dyn FnOnce() + Send>;

thread_local! {
    static TASKS: RefCell<Vec<Task>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn defer(task: impl FnOnce() + Send + 'static) {
    TASKS.with_borrow_mut(|tasks| tasks.push(Box::new(task)));
}

pub(crate) fn finish() {
    let tasks = TASKS.take();
    std::thread::scope(|scope| {
        let workers: Vec<_> = tasks.into_iter().map(|task| scope.spawn(task)).collect();
        for worker in workers {
            if worker.join().is_err() {
                eprintln!("Atlas Forge shutdown worker stopped unexpectedly");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    use std::time::Duration;

    #[test]
    fn shutdown_tasks_wait_for_the_window_and_can_finish_in_parallel() {
        let completed = Arc::new(AtomicBool::new(false));
        let marker = completed.clone();
        let (send, receive) = mpsc::channel();
        defer(move || {
            let parallel = receive.recv_timeout(Duration::from_secs(2)).is_ok();
            marker.store(parallel, Ordering::Relaxed);
        });
        defer(move || {
            send.send(()).unwrap();
        });
        assert!(!completed.load(Ordering::Relaxed));
        finish();
        assert!(completed.load(Ordering::Relaxed));
    }
}
