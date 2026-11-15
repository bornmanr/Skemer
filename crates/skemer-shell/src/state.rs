use std::process::Child;
use std::sync::Mutex;

use skemer_core::Settings;
use tokio::sync::Notify;

/// Shared shell state. The parent tray process owns the settings and the
/// periodic checker; the window process owns the same settings for its view.
pub struct AppState {
    settings: Mutex<Settings>,
    /// The `skemer --window` child, when the tray spawned one.
    window_child: Mutex<Option<Child>>,
    pub settings_changed: Notify,
}

impl AppState {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings: Mutex::new(settings),
            window_child: Mutex::new(None),
            settings_changed: Notify::new(),
        }
    }

    pub fn settings(&self) -> Settings {
        self.settings
            .lock()
            .map(|settings| settings.clone())
            .unwrap_or_default()
    }

    pub fn set_settings(&self, settings: Settings) {
        if let Ok(mut current) = self.settings.lock() {
            *current = settings;
        }
    }

    pub fn close_to_tray(&self) -> bool {
        self.settings().close_to_tray
    }

    /// True when a window child is still running. Reaps an exited child.
    pub fn window_open(&self) -> bool {
        let Ok(mut slot) = self.window_child.lock() else {
            return false;
        };
        let Some(child) = slot.as_mut() else {
            return false;
        };
        match child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => {
                *slot = None;
                false
            }
        }
    }

    pub fn set_window_child(&self, child: Option<Child>) {
        if let Ok(mut slot) = self.window_child.lock() {
            *slot = child;
        }
    }

    /// PID of the running window child, if any.
    pub fn window_child_pid(&self) -> Option<u32> {
        let mut slot = self.window_child.lock().ok()?;
        let child = slot.as_mut()?;
        match child.try_wait() {
            Ok(None) => Some(child.id()),
            _ => {
                *slot = None;
                None
            }
        }
    }

    pub fn take_window_child(&self) -> Option<Child> {
        self.window_child
            .lock()
            .ok()
            .and_then(|mut slot| slot.take())
    }
}

/// Send a signal to the tray parent when running as `skemer --window`.
pub fn signal_parent(signal: i32) {
    let Ok(pid) = std::env::var("SKEMER_PARENT_PID") else {
        return;
    };
    let Ok(pid) = pid.parse::<i32>() else {
        return;
    };
    unsafe {
        libc::kill(pid, signal);
    }
}
