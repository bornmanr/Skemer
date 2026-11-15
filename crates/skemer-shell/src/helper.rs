use anyhow::{Context, Result};
use futures_util::StreamExt;
use skemer_core::models::{ApplyOutcome, ApplyRequest, HoldDecision, UpdatePlan, UpdateSource};
use tauri::Emitter;

#[zbus::proxy(
    default_service = "org.veldmuislinux.skemer",
    default_path = "/org/veldmuislinux/skemer",
    interface = "org.veldmuislinux.skemer",
    gen_blocking = true
)]
trait SkemerHelper {
    async fn check(&self) -> zbus::Result<String>;
    async fn prepare(&self, request: String) -> zbus::Result<String>;
    async fn apply(&self, request: String) -> zbus::Result<String>;
    async fn cancel(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn progress(&self, percent: u32, message: String) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn log_line(&self, line: String) -> zbus::Result<()>;
}

/// Read the update plan from the privileged helper.
pub async fn check() -> Result<UpdatePlan> {
    let connection = zbus::Connection::system()
        .await
        .context("connect to the system bus")?;
    let proxy = SkemerHelperProxy::new(&connection)
        .await
        .context("connect to skemer-helper")?;
    let reply = proxy.check().await.context("call skemer-helper Check")?;
    serde_json::from_str(&reply).context("parse the update plan")
}

/// Validate a hold set with libalpm.
pub async fn prepare(holds: Vec<String>) -> Result<HoldDecision> {
    let request = ApplyRequest {
        sources: vec![UpdateSource::System],
        holds,
    };
    let request = serde_json::to_string(&request)?;
    let connection = zbus::Connection::system()
        .await
        .context("connect to the system bus")?;
    let proxy = SkemerHelperProxy::new(&connection)
        .await
        .context("connect to skemer-helper")?;
    let reply = proxy
        .prepare(request)
        .await
        .context("call skemer-helper Prepare")?;
    serde_json::from_str(&reply).context("parse the hold decision")
}

/// Apply the selected sources and forward progress/log signals to the UI.
pub async fn apply(app: tauri::AppHandle, request: ApplyRequest) -> Result<ApplyOutcome> {
    let connection = zbus::Connection::system()
        .await
        .context("connect to the system bus")?;
    let proxy = SkemerHelperProxy::new(&connection)
        .await
        .context("connect to skemer-helper")?;

    // Subscribe before Apply so the first lines are not lost.
    let mut progress = proxy
        .receive_progress()
        .await
        .context("listen for progress")?;
    let mut log = proxy
        .receive_log_line()
        .await
        .context("listen for log lines")?;

    let request = serde_json::to_string(&request)?;
    let apply = proxy.apply(request);
    tokio::pin!(apply);

    loop {
        tokio::select! {
            result = &mut apply => {
                let reply = result.context("call skemer-helper Apply")?;
                return serde_json::from_str(&reply).context("parse the apply outcome");
            }
            Some(message) = progress.next() => {
                if let Ok(args) = message.args() {
                    let _ = app.emit(
                        "skemer://update-progress",
                        serde_json::json!({
                            "percent": args.percent,
                            "message": args.message,
                        }),
                    );
                }
            }
            Some(message) = log.next() => {
                if let Ok(args) = message.args() {
                    let _ = app.emit(
                        "skemer://update-log",
                        serde_json::json!({
                            "line": args.line,
                        }),
                    );
                }
            }
        }
    }
}

/// Ask the helper to stop the running update.
pub async fn cancel() -> Result<()> {
    let connection = zbus::Connection::system()
        .await
        .context("connect to the system bus")?;
    let proxy = SkemerHelperProxy::new(&connection)
        .await
        .context("connect to skemer-helper")?;
    proxy.cancel().await.context("call skemer-helper Cancel")
}

/// Blocking plan read for the CLI.
pub fn check_blocking() -> Result<UpdatePlan> {
    let connection = zbus::blocking::Connection::system().context("connect to the system bus")?;
    let proxy = SkemerHelperProxyBlocking::new(&connection).context("connect to skemer-helper")?;
    let reply = proxy.check().context("call skemer-helper Check")?;
    serde_json::from_str(&reply).context("parse the update plan")
}
