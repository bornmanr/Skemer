# Skemer

Skemer is the updater for Veldmuis: a small tray application that reviews and
applies system and Flatpak updates, with a Veldmuis profile and theme.

The name is Afrikaans for dusk. The app is not tied to one package manager;
the first release covers the host system and Flatpak, with AUR planned as a
separate, user-run source.

## Status

This repository is at the scaffold stage. The design is in
[`docs/design.md`](docs/design.md); the UI reference is
[`docs/mockup.html`](docs/mockup.html).

## Shape

```text
crates/skemer-core/    models, settings, plan and hold helpers, source traits
crates/skemer-shell/   Tauri 2 window, tray, CLI routing, notifications
crates/skemer-helper/  privileged D-Bus service
src-ui/                TypeScript + Vite frontend
data/                  desktop entry, D-Bus service, polkit policy, icons
docs/                  design and UI reference
tests/fixtures/        shared IPC contract
```

The app follows the same tools as muis: Rust, Tauri 2, WebKitGTK, and a
TypeScript/Vite frontend.

## Build

The Veldmuis/Arch build dependencies include:

```text
rust (stable)
nodejs npm
webkit2gtk-4.1
gtk3
libayatana-appindicator
libalpm
```

Then:

```sh
cargo build --workspace
cargo test --workspace

npm ci --prefix src-ui
npm run build --prefix src-ui
npm test --prefix src-ui
```

The desktop window is created on demand. `skemer --tray` starts in the tray;
`skemer update` is the CLI path and `veldmuis-update` is an alias for it.

## Design boundaries

- The GUI never runs as root.
- The privileged helper is a small D-Bus service with fixed methods.
- libalpm is used for metadata, plans, and hold validation.
- `pacman -Syu` performs the actual system update, so hooks behave exactly as
  they do in pacman.
- System updates are atomic. A failing package can be held for one transaction,
  with explicit confirmation and a visible held state.
- “Reboot suggested”, never “required”.
