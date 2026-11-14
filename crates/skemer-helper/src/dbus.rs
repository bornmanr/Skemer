use anyhow::Result;
use skemer_core::models::{ApplyRequest, HoldDecision, UpdatePlan};
use zbus::{connection, interface};

use crate::{polkit, system};

pub struct SkemerService;

#[interface(name = "org.veldmuislinux.skemer")]
impl SkemerService {
    /// Read-only plan. This is allowed for local users and must never apply
    /// anything.
    async fn check(&self) -> Result<String, zbus::fdo::Error> {
        let plan = UpdatePlan::default();
        serde_json::to_string(&plan).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    /// Validate a proposed hold with libalpm. The helper does this before
    /// offering "hold and retry" in the UI.
    async fn prepare(&self, request: String) -> Result<String, zbus::fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
        let decision = HoldDecision::safe(format!(
            "hold validation is not implemented yet ({} holds)",
            request.holds.len()
        ));
        serde_json::to_string(&decision)
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    /// Apply the selected sources. This must be authorized by polkit before
    /// the actual work begins.
    async fn apply(&self, request: String) -> Result<String, zbus::fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| zbus::fdo::Error::InvalidArgs(error.to_string()))?;
        polkit::authorize_apply().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        let outcome =
            system::apply(&request).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
        serde_json::to_string(&outcome).map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }

    async fn cancel(&self) -> Result<(), zbus::fdo::Error> {
        Ok(())
    }

    async fn refresh_mirrors(&self) -> Result<(), zbus::fdo::Error> {
        Ok(())
    }
}

pub async fn serve() -> Result<()> {
    let _connection = connection::Builder::system()?
        .name("org.veldmuislinux.skemer")?
        .serve_at("/org/veldmuislinux/skemer", SkemerService)?
        .build()
        .await?;

    tracing::info!("skemer-helper is listening on the system bus");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
