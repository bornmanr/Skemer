# Packaging Notes

Skemer is delivered as a separate signed release tarball, like `muis`.

## Release Tarball

The `release` workflow builds in an Arch container so the binary links against
the same libalpm ABI as Veldmuis. It produces:

```text
skemer-<version>-x86_64-linux/
  skemer
  skemer-helper
  skemer.desktop
  skemer-autostart.desktop
  skemer-helper.service
  org.veldmuislinux.skemer.service
  org.veldmuislinux.skemer.conf
  org.veldmuislinux.skemer.policy
  icons/skemer.png
  LICENSE
```

The tarball, `SHA256SUMS`, and their detached signatures are published on the
GitHub release. The public key is attached too.

The workflow reads the version from `crates/skemer-shell/tauri.conf.json` and
refuses a tag that does not match it.

## Release Key

The Skemer release key is separate from muis and Veldmuis. See
[`keys/README.md`](keys/README.md) for creation, verification, and rotation.
For a guided setup, run `packaging/setup-release-key.sh` from a terminal. It
uses an isolated keyring, takes the passphrase from you, writes a full-secret
and revocation backup plus a zip of those two files, writes the signing
subkey/passphrase/fingerprint for GitHub, and copies the public key material
into both checkouts. It verifies the full secret and the signing subkey before
finishing and never commits or pushes.

The release job fails with a clear message until `keys/skemer-release.gpg`,
`keys/skemer-release.asc`, and `keys/skemer-trusted` exist and the
`SKEMER_GPG_PRIVATE_KEY`, `SKEMER_GPG_PASSPHRASE`, and `SKEMER_GPG_FPR` secrets
are configured in the `release` environment.

## libalpm ABI

`libalpm-soname` pins the soname the package is built against (currently `16`).
The release workflow reads the Arch container's `pacman` provides and refuses to
build when it does not match. The `libalpm ABI Watch` workflow checks daily and
opens an issue when Arch moves to a new soname.

When the soname changes:

1. Update `libalpm-soname`.
2. Rebuild and tag a new Skemer release.
3. Update the `libalpm.so=<N>-64` dependency in the Veldmuis package.

The package dependency intentionally blocks a pacman upgrade that would leave
Skemer linked against the old ABI until the rebuilt package is available.

## Veldmuis Package

The Veldmuis package is `veldmuis-skemer`. It should install:

```text
/usr/bin/skemer
/usr/bin/veldmuis-update -> skemer
/usr/lib/skemer/skemer-helper
/usr/lib/systemd/system/skemer-helper.service
/usr/share/dbus-1/system-services/org.veldmuislinux.skemer.service
/usr/share/dbus-1/system.d/org.veldmuislinux.skemer.conf
/usr/share/polkit-1/actions/org.veldmuislinux.skemer.policy
/usr/share/applications/skemer.desktop
/etc/xdg/autostart/skemer.desktop
/usr/share/icons/hicolor/256x256/apps/skemer.png
```

The Veldmuis profile and theme are supplied by the package. The upstream
default theme is neutral.

## Dependencies

Runtime:

```text
polkit
pacman
gtk3
webkit2gtk-4.1
libayatana-appindicator
libalpm.so=16-64
veldmuis-mirrorlist
```

Optional:

```text
flatpak
```

The D-Bus policy in `data/org.veldmuislinux.skemer.conf` lets root own the
service name and lets local users send read-only requests. The helper itself
checks polkit before `Apply` or `RefreshMirrors`.

## Release Pin

- The Veldmuis `Skemer Release Watch` workflow resolves the newest signed Skemer
  release and proposes a `veldmuis-skemer` pin.
- The resolver verifies `SHA256SUMS` and the tarball against the vendored
  `development/skemer-keyring/` key before changing the PKGBUILD.
- The Veldmuis PKGBUILD starts with a zero checksum placeholder; the first
  signed Skemer release replaces it with the verified checksum.

Do not commit private keys or recovery material to this repository.
