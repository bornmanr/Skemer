use std::error::Error;

use skemer_core::UpdatePlan;
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder,
};

use crate::state::AppState;

/// Set up the tray in the parent process. The window is a separate
/// `skemer --window` child so closing it releases every WebKitGTK process.
pub fn setup(app: &App) -> Result<(), Box<dyn Error>> {
    let open = MenuItem::with_id(app, "open", "Open Skemer", true, None::<&str>)?;
    let check = MenuItem::with_id(app, "check", "Check for updates", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &check, &quit])?;

    let icon = Image::from_bytes(include_bytes!("../icons/skemer.png"))?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("Skemer")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => open_window(app),
            "check" => {
                if app.state::<AppState>().window_open() {
                    open_window(app);
                } else {
                    crate::checker::check_now(app);
                }
            }
            "quit" => {
                quit_child(app);
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                open_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

/// Spawn the window child, or raise it if it is already running.
pub fn open_window(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Some(pid) = state.window_child_pid() {
        unsafe {
            libc::kill(pid as i32, libc::SIGUSR2);
        }
        return;
    }

    let Ok(executable) = std::env::current_exe() else {
        tracing::warn!("could not find the Skemer executable");
        return;
    };
    let child = std::process::Command::new(executable)
        .arg("--window")
        .env("SKEMER_PARENT_PID", std::process::id().to_string())
        .spawn();
    match child {
        Ok(child) => state.set_window_child(Some(child)),
        Err(error) => tracing::warn!("could not open the window: {error}"),
    }
}

fn quit_child(app: &AppHandle) {
    if let Some(mut child) = app.state::<AppState>().take_window_child() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Create the main window in the `skemer --window` child process.
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }

    let handle = app.clone();
    if let Ok(window) = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Skemer")
        .inner_size(960.0, 640.0)
        .min_inner_size(760.0, 520.0)
        .build()
    {
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                // The window process exits after this event. If close-to-tray
                // is off, close the tray parent too.
                if !handle.state::<AppState>().close_to_tray() {
                    crate::state::signal_parent(libc::SIGTERM);
                }
            }
        });
    }
}

/// Focus the window when the tray asks the existing child to come forward.
pub fn spawn_focus_handler(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2())
        else {
            return;
        };
        while signal.recv().await.is_some() {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }
    });
}

pub fn update_tooltip(app: &AppHandle, plan: &UpdatePlan) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    let tooltip = if plan.items.is_empty() {
        "Skemer — up to date".to_string()
    } else {
        format!(
            "Skemer — {} update{}",
            plan.items.len(),
            if plan.items.len() == 1 { "" } else { "s" }
        )
    };
    let _ = tray.set_tooltip(Some(tooltip));
}
