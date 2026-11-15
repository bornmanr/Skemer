use std::path::PathBuf;

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn xdg_dir(variable: &str, fallback: &str) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(variable) {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            return Some(path);
        }
    }
    home_dir().map(|home| home.join(fallback))
}

pub fn config_dir() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|path| path.join("skemer"))
}

pub fn state_dir() -> Option<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state").map(|path| path.join("skemer"))
}

pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|path| path.join("settings.json"))
}

pub fn state_file() -> Option<PathBuf> {
    state_dir().map(|path| path.join("state.json"))
}

pub fn log_file() -> Option<PathBuf> {
    state_dir().map(|path| path.join("skemer.log"))
}
