use crate::models::UpdateItem;

pub fn is_reboot_package(name: &str) -> bool {
    matches!(
        name,
        "linux"
            | "linux-lts"
            | "linux-zen"
            | "linux-hardened"
            | "linux-rt"
            | "linux-firmware"
            | "systemd"
            | "mkinitcpio"
            | "mesa"
            | "nvidia"
            | "nvidia-utils"
            | "nvidia-dkms"
            | "nvidia-open"
            | "nvidia-open-dkms"
            | "dkms"
    ) || name.starts_with("nvidia-580xx-")
}

/// Conservative reboot recommendation. The running kernel's module directory
/// disappearing is a strong signal; known in-use packages are a useful hint.
pub fn reboot_suggested(items: &[UpdateItem], running_kernel_modules_present: bool) -> bool {
    if !running_kernel_modules_present {
        return true;
    }
    items.iter().any(|item| is_reboot_package(&item.name))
}

/// Extract likely package names from a pacman error. This is a hint for the
/// UI, not a trust decision; the helper still validates any hold with libalpm.
pub fn hold_candidates_from_error(error: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;

    for character in error.chars() {
        if character == '\'' {
            if in_quote {
                if is_package_name(&current) && !candidates.contains(&current) {
                    candidates.push(current.clone());
                }
                current.clear();
                in_quote = false;
            } else {
                in_quote = true;
                current.clear();
            }
        } else if in_quote {
            current.push(character);
        }
    }

    candidates.sort();
    candidates
}

fn is_package_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains('/')
        && !value.chars().any(char::is_whitespace)
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '@' | '.' | '_' | '+' | '-')
        })
}

/// True when the running kernel still has its module directory. A false value
/// is a strong hint that the kernel package was replaced under the running
/// system and a reboot is worth suggesting.
pub fn running_kernel_modules_present(running_kernel: &str) -> bool {
    std::path::Path::new("/usr/lib/modules")
        .join(running_kernel)
        .is_dir()
}

/// The running kernel release, read from `/proc`.
pub fn running_kernel() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|release| release.trim().to_string())
        .filter(|release| !release.is_empty())
}

/// Reboot recommendation for the current machine.
pub fn reboot_suggested_now(items: &[UpdateItem]) -> bool {
    let modules_present = running_kernel()
        .map(|kernel| running_kernel_modules_present(&kernel))
        .unwrap_or(true);
    reboot_suggested(items, modules_present)
}

/// Parse a pacman transaction line such as `(3/12) upgrading linux` into a
/// percentage and a short message. Lines without a transaction counter return
/// `None`.
pub fn parse_progress(line: &str) -> Option<(u32, String)> {
    let line = line.trim();
    let rest = line.strip_prefix('(')?;
    let (fraction, message) = rest.split_once(')')?;
    let (current, total) = fraction.split_once('/')?;
    let current: u32 = current.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    if total == 0 || current > total {
        return None;
    }
    let message = message.trim();
    if message.is_empty() {
        return None;
    }
    Some((current * 100 / total, message.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{UpdateItem, UpdatePlan, UpdateSource};

    fn item(name: &str) -> UpdateItem {
        UpdateItem {
            id: name.to_string(),
            source: UpdateSource::System,
            name: name.to_string(),
            repo: "core".to_string(),
            current_version: "1".to_string(),
            new_version: "2".to_string(),
            download_bytes: 0,
            installed_bytes: 0,
            summary: String::new(),
            reboot_suggested: false,
            warning: None,
        }
    }

    #[test]
    fn kernel_and_graphics_suggest_reboot() {
        assert!(reboot_suggested(&[item("linux")], true));
        assert!(reboot_suggested(&[item("mesa")], true));
        assert!(reboot_suggested(&[item("nvidia-580xx-utils")], true));
    }

    #[test]
    fn unrelated_packages_do_not_suggest_reboot() {
        assert!(!reboot_suggested(&[item("firefox"), item("git")], true));
    }

    #[test]
    fn missing_running_kernel_modules_suggest_reboot() {
        assert!(reboot_suggested(&[item("firefox")], false));
    }

    #[test]
    fn quoted_package_names_are_extracted() {
        let error = "error: failed to commit transaction (conflicting files)\n\
                     /usr/lib/foo: /usr/lib/foo exists in both 'mpv' and 'mpv-old'";
        assert_eq!(
            hold_candidates_from_error(error),
            vec!["mpv".to_string(), "mpv-old".to_string()]
        );
    }

    #[test]
    fn file_paths_are_not_treated_as_packages() {
        let error = "error: failed to commit transaction (conflicting files)\n\
                     /usr/lib/foo: /usr/lib/foo exists in both 'mpv' and '/usr/lib/foo'";
        assert_eq!(hold_candidates_from_error(error), vec!["mpv".to_string()]);
    }

    #[test]
    fn empty_error_has_no_candidates() {
        assert!(hold_candidates_from_error("").is_empty());
    }

    #[test]
    fn plan_totals_and_sources() {
        let plan = UpdatePlan::from_items(vec![item("linux"), item("firefox")], "now");
        assert_eq!(plan.items.len(), 2);
        assert_eq!(plan.total_download_bytes, 0);
        assert_eq!(plan.source_count(UpdateSource::System), 2);
        assert_eq!(plan.sources(), vec![UpdateSource::System]);
    }

    #[test]
    fn pacman_progress_lines_are_parsed() {
        assert_eq!(
            parse_progress("(3/12) upgrading linux"),
            Some((25, "upgrading linux".to_string()))
        );
        assert_eq!(
            parse_progress("( 1/10) installing mesa"),
            Some((10, "installing mesa".to_string()))
        );
        assert_eq!(
            parse_progress("(12/12) checking keys in keyring"),
            Some((100, "checking keys in keyring".to_string()))
        );
    }

    #[test]
    fn non_progress_lines_are_ignored() {
        assert_eq!(parse_progress(":: Starting full system upgrade..."), None);
        assert_eq!(parse_progress("(1/0) broken"), None);
        assert_eq!(parse_progress("(2/1) broken"), None);
        assert_eq!(parse_progress("(1/2)"), None);
        assert_eq!(parse_progress(""), None);
    }
}
