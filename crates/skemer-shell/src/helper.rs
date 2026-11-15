use anyhow::{Context, Result};
use skemer_core::models::{ApplyOutcome, ApplyRequest, HoldDecision, UpdatePlan};

const SERVICE: &str = "org.veldmuislinux.skemer";
const PATH: &str = "/org/veldmuislinux/skemer";
const INTERFACE: &str = "org.veldmuislinux.skemer";

async fn proxy() -> Result<zbus::Proxy<'static>> {
    let connection = zbus::Connection::system()
        .await
        .context("connect to the system bus")?;
    zbus::Proxy::new_owned(connection, SERVICE, PATH, INTERFACE)
        .await
        .context("connect to skemer-helper")
}

/// Read the update plan from the privileged helper.
pub async fn check() -> Result<UpdatePlan> {
    let reply: String = proxy()
        .await?
        .call("Check", &())
        .await
        .context("call skemer-helper Check")?;
    serde_json::from_str(&reply).context("parse the update plan")
}

/// Validate a hold set with libalpm.
pub async fn prepare(holds: Vec<String>) -> Result<HoldDecision> {
    let request = ApplyRequest {
        sources: vec![skemer_core::UpdateSource::System],
        holds,
    };
    let request = serde_json::to_string(&request)?;
    let reply: String = proxy()
        .await?
        .call("Prepare", &(request,))
        .await
        .context("call skemer-helper Prepare")?;
    serde_json::from_str(&reply).context("parse the hold decision")
}

/// Apply the selected sources through the privileged helper.
pub async fn apply(request: &ApplyRequest) -> Result<ApplyOutcome> {
    let request = serde_json::to_string(request)?;
    let reply: String = proxy()
        .await?
        .call("Apply", &(request,))
        .await
        .context("call skemer-helper Apply")?;
    serde_json::from_str(&reply).context("parse the apply outcome")
}

/// Blocking plan read for the CLI.
pub fn check_blocking() -> Result<UpdatePlan> {
    let connection = zbus::blocking::Connection::system().context("connect to the system bus")?;
    let proxy = zbus::blocking::Proxy::new(&connection, SERVICE, PATH, INTERFACE)
        .context("connect to skemer-helper")?;
    let reply: String = proxy
        .call("Check", &())
        .context("call skemer-helper Check")?;
    serde_json::from_str(&reply).context("parse the update plan")
}
