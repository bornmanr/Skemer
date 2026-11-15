use std::path::Path;

use skemer_core::{Settings, UpdatePlan};

use crate::helper;

/// Returns true when the command was handled by the CLI and the GUI should not
/// start. `--tray` is intentionally not handled here; it is the GUI tray mode.
pub fn try_run() -> bool {
    let mut arguments = std::env::args();
    let program = arguments.next().unwrap_or_default();
    let invoked_as_update = Path::new(&program)
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "veldmuis-update");

    if invoked_as_update {
        run_update(arguments.collect());
        return true;
    }

    let Some(command) = arguments.next() else {
        return false;
    };

    match command.as_str() {
        "check" => {
            run_check(arguments.collect());
            true
        }
        "update" => {
            run_update(arguments.collect());
            true
        }
        "--help" | "-h" => {
            print_help();
            true
        }
        "--version" | "-V" => {
            println!("skemer {}", env!("CARGO_PKG_VERSION"));
            true
        }
        "--tray" => false,
        other => {
            eprintln!("skemer: unknown command: {other}");
            print_help();
            std::process::exit(2);
        }
    }
}

fn run_check(arguments: Vec<String>) {
    let json = arguments.iter().any(|argument| argument == "--json");
    let settings = Settings::load().unwrap_or_default();
    match helper::check_blocking() {
        Ok(mut plan) => {
            let warnings = crate::flatpak::add_flatpak(&mut plan, &settings);
            plan.warnings.extend(warnings);
            print_plan(&plan, json);
        }
        Err(error) => {
            eprintln!("skemer: {error:#}");
            eprintln!(
                "skemer: the helper may not be running; try `systemctl start skemer-helper`."
            );
            std::process::exit(1);
        }
    }
}

fn print_plan(plan: &UpdatePlan, json: bool) {
    if json {
        match serde_json::to_string_pretty(plan) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("skemer: {error}"),
        }
        return;
    }

    if plan.items.is_empty() {
        println!("No updates are pending.");
        return;
    }

    println!(
        "{} update{} pending ({} to download)",
        plan.items.len(),
        if plan.items.len() == 1 { "" } else { "s" },
        format_bytes(plan.total_download_bytes)
    );
    for item in &plan.items {
        println!(
            "  {:<8} {:<28} {} -> {}",
            item.repo, item.name, item.current_version, item.new_version
        );
    }
    if plan.reboot_suggested {
        println!("Reboot suggested.");
    }
}

fn run_update(arguments: Vec<String>) {
    if !is_root() {
        eprintln!("skemer: update must run as root; use the Skemer app for polkit authorization.");
        std::process::exit(1);
    }

    let holds = match parse_holds(&arguments) {
        Ok(holds) => holds,
        Err(error) => {
            eprintln!("skemer: {error}");
            std::process::exit(2);
        }
    };

    let mut command = std::process::Command::new("pacman");
    command.arg("-Syu");
    if !holds.is_empty() {
        command.arg("--ignore").arg(holds.join(","));
    }
    let status = command.status();
    match status {
        Ok(status) if status.success() => {}
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(error) => {
            eprintln!("skemer: could not run pacman: {error}");
            std::process::exit(1);
        }
    }
}

fn parse_holds(arguments: &[String]) -> Result<Vec<String>, String> {
    let mut holds = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        match arguments[index].as_str() {
            "--hold" => {
                index += 1;
                let Some(package) = arguments.get(index) else {
                    return Err("--hold needs a package name".to_string());
                };
                if !holds.iter().any(|hold| hold == package) {
                    holds.push(package.clone());
                }
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    Ok(holds)
}

fn is_root() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find(|line| line.starts_with("Uid:"))
                .and_then(|line| line.split_whitespace().nth(2).map(str::to_string))
        })
        .is_some_and(|uid| uid == "0")
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    let bytes = bytes as f64;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes / GIB)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else {
        format!("{:.0} KiB", bytes / 1024.0)
    }
}

fn print_help() {
    println!(
        "Skemer {}\n\n\
         Usage:\n\
         \x20 skemer                       Open the updater\n\
         \x20 skemer --tray                Start in the tray\n\
         \x20 skemer check [--json]        Show the update plan\n\
         \x20 skemer update [--hold PKG]   Apply updates (root)\n\
         \x20 skemer --version\n\
         \x20 skemer --help",
        env!("CARGO_PKG_VERSION")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_are_parsed_and_deduplicated() {
        let arguments = vec![
            "--hold".to_string(),
            "linux".to_string(),
            "--hold".to_string(),
            "mesa".to_string(),
            "--hold".to_string(),
            "linux".to_string(),
        ];
        assert_eq!(
            parse_holds(&arguments).unwrap(),
            vec!["linux".to_string(), "mesa".to_string()]
        );
    }

    #[test]
    fn hold_without_a_value_is_an_error() {
        assert!(parse_holds(&["--hold".to_string()]).is_err());
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        assert!(parse_holds(&["--force".to_string()]).is_err());
    }
}
