# Packaging Notes

Skemer is delivered as a separate signed release tarball, like `muis`.

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
/usr/share/icons/hicolor/.../apps/skemer.png
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

`libalpm.so=16-64` is provided by pacman. It is pinned so a prebuilt Skemer is
only installed against a compatible libalpm. The ABI watcher triggers a new
Skemer release when the soname changes; Skemer and libalpm upgrade atomically
in `pacman -Syu`.

The D-Bus policy in `data/org.veldmuislinux.skemer.conf` lets root own the
service name and lets local users send read-only requests. The helper itself
checks polkit before `Apply` or `RefreshMirrors`.

## Release

- Skemer releases are signed with the Skemer release key.
- Veldmuis vendors the public key and pins the tarball checksum.
- A `Skemer Release Watch` workflow proposes the next pin.
- A libalpm ABI watcher opens an issue/PR when the pacman soname changes.

Do not commit private keys or recovery material to this repository.
