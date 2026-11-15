# Skemer Design

Status: agreed design for the first release  
UI reference: [`mockup.html`](mockup.html) · [PNG](mockup.png)

## 1. What Skemer Is

Skemer is the Veldmuis updater: a small tray application that reviews and
applies system and Flatpak updates. It is built like muis, with Rust, Tauri 2,
WebKitGTK, and a TypeScript/Vite frontend, but it is not tied to Veldmuis.
Veldmuis ships the branded profile and theme; other Arch systems can use the
generic profile.

The first release covers:

- the host system (`pacman -Syu`);
- Flatpak system updates;
- Flatpak user updates.

AUR is deliberately deferred. It will be a separate, user-run source with a
PTY-backed worker and a persistent user-supported warning.

## 2. Product Behavior

- Tray application with autostart.
- Default check interval: 6 hours (configurable: 1/3/6/12/24 hours, or never).
- Desktop notifications on by default.
- Close to tray by default; tray “Quit” exits.
- The main window is created on demand. It runs in a short-lived
  `skemer --window` child process; closing it exits that process and releases
  WebKitGTK. The tray and periodic checker live in the Rust parent process.
- Window: 960 × 640 default, 760 × 520 minimum, not maximized.
- “Reboot suggested”, never “required”. Amber chip, not red.

## 3. Sources

### System

- **libalpm** (official `alpm` crate 5.x) for metadata, plans, transaction
  preparation, conflict detection, and hold validation.
- **`pacman -Syu`** for the actual apply, so pacman hooks and transaction
  behavior are exactly pacman’s.
- System updates are atomic. No per-package official updates.
- A failing package can be held for one transaction, but only when libalpm can
  prepare the transaction with that package ignored.
- The hold is explicit, visible, and per-transaction. It is never persisted as
  `IgnorePkg`, and it is never a silent skip.
- After a successful hold-and-retry, the held package is shown as “Held” and a
  “Retry held” action is available.

### Flatpak

Two sources:

- **Flatpak system** — via the privileged D-Bus helper, polkit-authorized.
- **Flatpak user** — runs as the desktop user, no prompt.

One Flatpak tab with two groups. Runtimes are included with the apps. “Update
all” applies system, then Flatpak system under the same polkit authorization,
then Flatpak user.

### AUR

Deferred. No AUR tab, no AUR code path, and no AUR privilege path in v1.

## 4. Privilege Model

```text
skemer --tray (user)
  ├─ tray, periodic checker, notifications (Rust)
  ├─ skemer --window (user, on demand)
  │    ├─ WebKitGTK window; exits when closed
  │    └─ Flatpak user updates (user)
  └─ D-Bus → org.veldmuislinux.skemer
       └─ skemer-helper (root, systemd/D-Bus activated)
            ├─ libalpm check/prepare
            ├─ pacman -Syu
            ├─ flatpak system update
            └─ veldmuis-refresh-arch-mirrors / reflector
```

- The GUI never runs as root.
- The helper exposes fixed methods only. No command strings, no arbitrary
  paths, no shell interpolation.
- `Check` is allowed for active local users; it is read-only.
- `Prepare` validates holds with libalpm; it is read-only.
- `Apply` requires polkit `auth_admin_keep`.
- `RefreshMirrors` requires polkit `auth_admin_keep`.
- `Cancel` is allowed for the caller that started the apply.
- One transaction at a time. The helper releases the pacman DB lock on exit.
- AUR helpers are never run by the helper.
- Logs go to the journal; the UI shows the streamed log.
- The tray parent and window child use small signals: SIGHUP reloads settings,
  SIGUSR1 refreshes the tray state, SIGUSR2 focuses an existing window, and
  SIGTERM closes the whole app when close-to-tray is off.

The D-Bus name is `org.veldmuislinux.skemer`; the polkit action is
`org.veldmuislinux.skemer.apply`.

## 5. Hold And Retry

1. `pacman -Syu` fails.
2. The helper returns the error and likely package candidates.
3. The UI stops; it does not silently continue.
4. If libalpm can prepare the transaction with one candidate ignored, the UI
   offers “Hold `<pkg>` and retry the rest”.
5. The user confirms; the hold is visible in the summary and the package list.
6. The helper runs `pacman -Syu --ignore <pkg>`.
7. If the hold fails, the UI shows the new error and does not loop.
8. The held package is listed as “Held”; “Retry held” is available later.
9. If the held package is the one the user actually cares about, the UI says
   plainly that it will not be updated.

No automatic holds. No permanent ignores. No silent exclusions.

## 6. Reboot

“Reboot suggested” is triggered by:

- kernel packages (`linux`, `linux-lts`, `linux-zen`, `linux-hardened`,
  `linux-rt`);
- `linux-firmware`;
- `systemd`;
- `mkinitcpio`/initramfs changes;
- `mesa`, NVIDIA drivers, DKMS;
- the running kernel’s module directory disappearing.

Graphics-only changes can say “Restart session suggested” instead of a full
reboot.

## 7. Identity And Branding

- Binary: `skemer`
- Display name: `Skemer`
- Tauri identifier: `org.veldmuislinux.skemer`
- Veldmuis package: `veldmuis-skemer`
- Desktop entry: `skemer.desktop`
- D-Bus: `org.veldmuislinux.skemer`
- Polkit: `org.veldmuislinux.skemer.apply`
- Helper: `skemer-helper`

The core is distro-agnostic. A `DistroProfile` selects:

- display name, desktop entry, icon;
- theme/colors;
- repository labels;
- mirror refresh command (`veldmuis-refresh-arch-mirrors` vs `reflector` or
  none);
- default sources.

The Veldmuis package supplies the Veldmuis profile and theme. Upstream ships a
neutral default. The layout is the same; branding is data, not hard-coded.

## 8. CLI

Skemer replaces the bash `veldmuis-update` wrapper.

- `skemer` — open the updater.
- `skemer --tray` — start in the tray.
- `skemer check [--json]` — show the plan.
- `skemer update [--hold PKG]` — apply updates; requires root.
- `veldmuis-update` — argv[0] alias to `skemer update`.

The CLI is the root/recovery path. The GUI is the polkit path. `skemer check`
asks the helper for a read-only plan; if the helper is unavailable it says so
and suggests starting `skemer-helper`.

## 9. Delivery

- Separate repository: `Skemer`.
- Separate Skemer release key; signed tarball; Veldmuis pin; release watcher.
- `veldmuis-skemer` packages the signed tarball like `veldmuis-muis`.
- libalpm ABI: the package depends on the exact `libalpm.so` soname. Skemer and
  libalpm upgrade atomically in `pacman -Syu`. An ABI watcher forces a new
  Skemer build when the soname changes.
- If the new Skemer build is not available, the dependency blocks the libalpm
  upgrade rather than leaving a broken Skemer.

## 10. Testing

- Rust unit tests for plan totals, hold candidates, reboot detection, Flatpak
  parsing, and settings.
- Fake `pacman`/`flatpak` binaries for apply/log parsing.
- D-Bus service tests on a private bus with a test policy.
- UI Vitest tests and Playwright interaction tests.
- One VM release gate for real system + Flatpak + polkit.

## 11. Phases

1. Scaffold: workspace, models, settings, plan helpers, source traits, tray
   skeleton, D-Bus skeleton, UI shell, docs, CI.
2. libalpm system source: check, prepare, hold validation.
3. D-Bus helper: polkit, pacman apply, log streaming, cancel.
4. Flatpak system/user: check, apply, grouping.
5. Tray: periodic checks, notifications, autostart, window lifecycle.
6. Packaging and release: Veldmuis package, keyring, release watcher, ABI
   watcher.
7. AUR: deferred follow-up.

## 12. Open Implementation Details

- The fixed D-Bus methods are `Check`, `Prepare`, `Apply`, `Cancel`, and
  `RefreshMirrors`. `Apply` emits `Progress` and `LogLine` signals while it
  runs, and `Cancel` sends SIGINT to the pacman process group. Only the user
  that started the update may cancel it.
- Flatpak runtime version display when the remote does not expose a version;
  the branch is shown in the summary today.
- Theme loader format (CSS variables + a small theme manifest).
- Log retention and copy-log behavior.
- Tray menu copy and icon states.
- Package dependency list for `veldmuis-skemer`.
- Release watcher and ABI watcher workflow.
