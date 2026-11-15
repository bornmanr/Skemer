use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A cancellable child process. The child should be started in its own process
/// group; `cancel` sends SIGINT to the group so helper processes stop too.
#[derive(Clone, Default)]
pub struct CancelHandle {
    cancelled: Arc<AtomicBool>,
    pid: Arc<Mutex<Option<u32>>>,
}

impl CancelHandle {
    /// Ask the process to stop. SIGINT lets pacman release its transaction and
    /// database lock cleanly.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let Ok(pid) = self.pid.lock() else {
            return;
        };
        let Some(pid) = *pid else {
            return;
        };
        if pid == 0 {
            return;
        }
        let pid = pid as i32;
        // The group signal is the important one; the direct signal covers a
        // child that did not get its own process group.
        unsafe {
            libc::kill(pid, libc::SIGINT);
            libc::kill(-pid, libc::SIGINT);
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn set_pid(&self, pid: Option<u32>) {
        if let Ok(mut current) = self.pid.lock() {
            *current = pid;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_sets_the_flag() {
        let handle = CancelHandle::default();
        assert!(!handle.is_cancelled());
        handle.cancel();
        assert!(handle.is_cancelled());
    }
}
