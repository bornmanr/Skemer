use anyhow::Result;

/// Check the caller against the polkit action `org.veldmuislinux.skemer.apply`.
/// The implementation will call `org.freedesktop.PolicyKit1.Authority` with
/// the caller's bus name and a `auth_admin_keep` action. For now this is a
/// scaffold placeholder.
pub fn authorize_apply() -> Result<()> {
    Ok(())
}
