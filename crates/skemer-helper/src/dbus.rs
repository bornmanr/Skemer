use anyhow::Result;
use skemer_core::models::ApplyRequest;
use zbus::message::Header;
use zbus::{connection, fdo, interface, Connection};

use crate::{polkit, system};

pub struct SkemerService {
    /// One libalpm transaction at a time. The lock covers check, hold
    /// validation, and apply so callers cannot race on the pacman database.
    transaction_lock: tokio::sync::Mutex<()>,
}

impl Default for SkemerService {
    fn default() -> Self {
        Self {
            transaction_lock: tokio::sync::Mutex::new(()),
        }
    }
}

#[interface(name = "org.veldmuislinux.skemer")]
impl SkemerService {
    /// Read-only plan. Allowed for local users and must never apply anything.
    async fn check(&self) -> Result<String, fdo::Error> {
        let _guard = self.transaction_lock.lock().await;
        let plan = tokio::task::spawn_blocking(system::check)
            .await
            .map_err(failed)?
            .map_err(failed)?;
        serde_json::to_string(&plan).map_err(failed)
    }

    /// Validate a proposed hold set with libalpm. This is read-only; the UI
    /// calls it before offering "hold and retry".
    async fn prepare(&self, request: String) -> Result<String, fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        let _guard = self.transaction_lock.lock().await;
        let decision = tokio::task::spawn_blocking(move || system::prepare(&request.holds))
            .await
            .map_err(failed)?
            .map_err(failed)?;
        serde_json::to_string(&decision).map_err(failed)
    }

    /// Apply the selected sources. Requires polkit authorization.
    async fn apply(
        &self,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
        request: String,
    ) -> Result<String, fdo::Error> {
        let request: ApplyRequest = serde_json::from_str(&request)
            .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))?;
        let sender = header
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| fdo::Error::Failed("missing D-Bus sender".to_string()))?;

        polkit::authorize(connection, &sender, polkit::APPLY_ACTION)
            .await
            .map_err(failed)?;

        let _guard = self.transaction_lock.lock().await;
        let outcome = system::apply(&request).await.map_err(failed)?;
        serde_json::to_string(&outcome).map_err(failed)
    }

    /// Cancel the running transaction. Streaming and interruption arrive with
    /// the progress phase; until then there is nothing to interrupt.
    async fn cancel(&self) -> Result<(), fdo::Error> {
        Ok(())
    }

    /// Refresh the Arch mirrorlist. This lands with the mirror phase.
    async fn refresh_mirrors(&self) -> Result<(), fdo::Error> {
        Err(fdo::Error::NotSupported(
            "mirror refresh is not implemented yet".to_string(),
        ))
    }
}

fn failed(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}

pub async fn serve() -> Result<()> {
    let _connection = connection::Builder::system()?
        .name("org.veldmuislinux.skemer")?
        .serve_at("/org/veldmuislinux/skemer", SkemerService::default())?
        .build()
        .await?;

    tracing::info!("skemer-helper is listening on the system bus");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use skemer_core::models::UpdatePlan;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs a session bus"]
    async fn check_round_trips_over_dbus() {
        let connection = connection::Builder::session()
            .expect("session bus")
            .build()
            .await
            .expect("connect");
        connection
            .object_server()
            .at("/org/veldmuislinux/skemer", SkemerService::default())
            .await
            .expect("serve the interface");

        let destination = connection.unique_name().expect("unique name").to_string();
        let proxy = zbus::Proxy::new(
            &connection,
            destination,
            "/org/veldmuislinux/skemer",
            "org.veldmuislinux.skemer",
        )
        .await
        .expect("proxy");

        let reply: String = proxy.call("Check", &()).await.expect("Check");
        let plan: UpdatePlan = serde_json::from_str(&reply).expect("plan JSON");
        assert!(!plan.checked_at.is_empty());
    }
}
