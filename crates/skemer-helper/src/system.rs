use anyhow::Result;
use skemer_core::models::{ApplyOutcome, ApplyRequest};

/// Apply the selected sources. The real implementation uses libalpm to prepare
/// and validate the transaction, then runs `pacman -Syu` with the held package
/// list. This is intentionally a stub in the scaffold.
pub fn apply(_request: &ApplyRequest) -> Result<ApplyOutcome> {
    anyhow::bail!("system apply is not implemented yet")
}
