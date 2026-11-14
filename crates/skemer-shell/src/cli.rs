use skemer_core::UpdatePlan;

/// Returns true when the command was handled by the CLI and the GUI should not
/// start. `--tray` is intentionally not handled here; it is the GUI tray mode.
pub fn try_run() -> bool {
    let mut arguments = std::env::args().skip(1);
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
            true
        }
    }
}

fn run_check(arguments: Vec<String>) {
    let json = arguments.iter().any(|argument| argument == "--json");
    let plan = UpdatePlan::default();

    if json {
        match serde_json::to_string_pretty(&plan) {
            Ok(text) => println!("{text}"),
            Err(error) => eprintln!("skemer: {error}"),
        }
    } else {
        println!("No update plan is available yet.");
    }
}

fn run_update(_arguments: Vec<String>) {
    eprintln!("skemer: update is not implemented yet");
    std::process::exit(1);
}

fn print_help() {
    println!(
        "Skemer {}\n\
         \n\
         Usage:\n\
           skemer                       Open the updater\n\
           skemer --tray                Start in the tray\n\
           skemer check [--json]        Show the update plan\n\
           skemer update [--hold PKG]   Apply updates\n\
           skemer --version\n\
           skemer --help",
        env!("CARGO_PKG_VERSION")
    );
}
