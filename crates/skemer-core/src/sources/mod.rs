pub mod flatpak;

pub use flatpak::{
    flatpak_available, parse_installed, parse_updates, FlatpakCli, FlatpakScope, FlatpakSource,
};
