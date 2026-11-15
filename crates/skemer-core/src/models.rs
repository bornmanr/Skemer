use serde::{Deserialize, Serialize};

/// One update source. The v1 UI exposes system, Flatpak system, and Flatpak
/// user. AUR is intentionally not part of the first release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateSource {
    System,
    FlatpakSystem,
    FlatpakUser,
}

impl UpdateSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::FlatpakSystem => "Flatpak system",
            Self::FlatpakUser => "Flatpak user",
        }
    }
}

impl std::str::FromStr for UpdateSource {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "system" => Ok(Self::System),
            "flatpak-system" => Ok(Self::FlatpakSystem),
            "flatpak-user" => Ok(Self::FlatpakUser),
            other => Err(format!("unknown update source: {other}")),
        }
    }
}

/// One pending package update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateItem {
    pub id: String,
    pub source: UpdateSource,
    pub name: String,
    pub repo: String,
    pub current_version: String,
    pub new_version: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub summary: String,
    pub reboot_suggested: bool,
    pub warning: Option<String>,
}

/// The complete update plan shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct UpdatePlan {
    pub items: Vec<UpdateItem>,
    pub total_download_bytes: u64,
    pub reboot_suggested: bool,
    pub checked_at: String,
    pub warnings: Vec<String>,
    /// Packages held for the current transaction. These are not persisted.
    pub held_packages: Vec<String>,
}

impl UpdatePlan {
    pub fn from_items(items: Vec<UpdateItem>, checked_at: impl Into<String>) -> Self {
        let total_download_bytes = items.iter().map(|item| item.download_bytes).sum();
        let reboot_suggested = items.iter().any(|item| item.reboot_suggested);
        Self {
            items,
            total_download_bytes,
            reboot_suggested,
            checked_at: checked_at.into(),
            warnings: Vec::new(),
            held_packages: Vec::new(),
        }
    }

    pub fn source_count(&self, source: UpdateSource) -> usize {
        self.items
            .iter()
            .filter(|item| item.source == source)
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn sources(&self) -> Vec<UpdateSource> {
        let mut sources: Vec<UpdateSource> = self.items.iter().map(|item| item.source).collect();
        sources.sort();
        sources.dedup();
        sources
    }
}

/// Request passed to the privileged helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyRequest {
    pub sources: Vec<UpdateSource>,
    /// Package names to hold for this transaction only. The helper validates
    /// these with libalpm before applying.
    #[serde(default)]
    pub holds: Vec<String>,
}

/// Result of an apply operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyOutcome {
    pub success: bool,
    #[serde(default)]
    pub cancelled: bool,
    pub held_packages: Vec<String>,
    pub reboot_suggested: bool,
    pub log_tail: String,
    /// Human-readable failure reason when `success` is false.
    #[serde(default)]
    pub error: Option<String>,
    /// Packages that look like the cause of a failure. These are hints for the
    /// UI; the helper validates any proposed hold with libalpm.
    #[serde(default)]
    pub candidates: Vec<String>,
}

/// Result of validating a proposed hold with libalpm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldDecision {
    pub safe: bool,
    pub reason: String,
    pub candidates: Vec<String>,
}

impl HoldDecision {
    pub fn safe(reason: impl Into<String>) -> Self {
        Self {
            safe: true,
            reason: reason.into(),
            candidates: Vec::new(),
        }
    }

    pub fn rejected(reason: impl Into<String>, candidates: Vec<String>) -> Self {
        Self {
            safe: false,
            reason: reason.into(),
            candidates,
        }
    }
}
