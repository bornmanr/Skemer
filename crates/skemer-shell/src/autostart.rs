use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use skemer_core::paths;

/// Write or hide the user-level autostart entry. A `Hidden=true` entry
/// overrides the system-wide file installed by the Veldmuis package.
pub fn apply(enabled: bool) -> Result<()> {
    let path = paths::autostart_file().context("no XDG config directory is available")?;
    let executable = if enabled {
        Some(std::env::current_exe().context("find the Skemer executable")?)
    } else {
        None
    };
    apply_at(&path, enabled, executable.as_deref())
}

fn apply_at(path: &Path, enabled: bool, executable: Option<&Path>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create autostart directory {}", parent.display()))?;
    }

    let contents = if enabled {
        let executable = executable.context("find the Skemer executable")?;
        desktop_entry(&executable.to_string_lossy())
    } else {
        hidden_entry()
    };

    fs::write(path, contents)
        .with_context(|| format!("write autostart file {}", path.display()))?;
    Ok(())
}

fn desktop_entry(executable: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Skemer\n\
         Comment=Check for Veldmuis and Flatpak updates\n\
         Exec=\"{executable}\" --tray\n\
         Icon=skemer\n\
         Terminal=false\n\
         NoDisplay=true\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

fn hidden_entry() -> String {
    "[Desktop Entry]\n\
     Type=Application\n\
     Name=Skemer\n\
     Comment=Check for Veldmuis and Flatpak updates\n\
     Exec=skemer --tray\n\
     Icon=skemer\n\
     Terminal=false\n\
     NoDisplay=true\n\
     Hidden=true\n"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabled_entry_points_at_the_current_binary() {
        let entry = desktop_entry("/usr/bin/skemer");
        assert!(entry.contains("Exec=\"/usr/bin/skemer\" --tray"));
        assert!(entry.contains("X-GNOME-Autostart-enabled=true"));
        assert!(!entry.contains("Hidden=true"));
    }

    #[test]
    fn disabled_entry_is_hidden() {
        let entry = hidden_entry();
        assert!(entry.contains("Hidden=true"));
        assert!(!entry.contains("X-GNOME-Autostart-enabled=true"));
    }

    #[test]
    fn apply_writes_both_forms() {
        let directory =
            std::env::temp_dir().join(format!("skemer-autostart-{}", std::process::id()));
        let path = directory.join("skemer.desktop");

        apply_at(&path, true, Some(Path::new("/usr/bin/skemer"))).expect("enable");
        let entry = fs::read_to_string(&path).expect("read enabled entry");
        assert!(entry.contains("Exec=\"/usr/bin/skemer\" --tray"));
        assert!(!entry.contains("Hidden=true"));

        apply_at(&path, false, None).expect("disable");
        let entry = fs::read_to_string(&path).expect("read hidden entry");
        assert!(entry.contains("Hidden=true"));

        fs::remove_dir_all(&directory).ok();
    }
}
