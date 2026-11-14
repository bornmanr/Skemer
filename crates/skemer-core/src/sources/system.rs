use anyhow::Result;

use crate::models::{HoldDecision, UpdatePlan};

/// Read-only system update source. The implementation will use `alpm` 5.x for
/// sync DB access, metadata, transaction preparation, and hold validation.
/// Applying is intentionally not part of this trait: `pacman -Syu` is the
/// apply engine so hooks and transaction behavior stay pacman's.
pub trait SystemSource {
    fn check(&self) -> Result<UpdatePlan>;
    fn prepare(&self, holds: &[String]) -> Result<HoldDecision>;
}

pub struct LibalpmSource;

impl SystemSource for LibalpmSource {
    fn check(&self) -> Result<UpdatePlan> {
        anyhow::bail!("libalpm check is not implemented yet")
    }

    fn prepare(&self, _holds: &[String]) -> Result<HoldDecision> {
        anyhow::bail!("libalpm prepare is not implemented yet")
    }
}
