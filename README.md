# strixctl

A power management GUI and D-Bus daemon for the ASUS ProArt PX13 (AMD Strix Halo) on Linux. Provides a native egui interface and GNOME Shell integration for switching platform profiles, tuning AMD CPU power limits (PPT), and editing fan curves — without running anything as root.

## Features

- **Platform profiles** — switch between Quiet, Balanced, and Performance via `asusctl`
- **AMD PPT tuning** — set STAPM / Fast / Slow power limits in mW via `ryzenadj`
- **Fan curve editor** — drag-and-drop curve with zoom, global shift, and hysteresis control
- **Named profiles** — save and restore any combination of PPT limits and fan curve to `~/.config/strixctl/profiles.json`
- **Live temperature** — background watcher updates the UI every second; auto-switches to Performance at ≥ 95 °C
- **D-Bus daemon** (`strixctld`) — exposes profiles and controls over a session bus interface
- **GNOME Shell extension** — Quick Settings panel with profile picker and configurable cycle keybinding (default `Super+P`)

## Requirements

| Tool | Purpose |
|------|---------|
| [`asusctl`](https://gitlab.com/asus-linux/asusctl) | Platform profile and fan curve control |
| [`ryzenadj`](https://github.com/FlyGoat/RyzenAdj) | AMD CPU PPT tuning |
| `pkexec` (polkit) | Privilege escalation for `ryzenadj` |
| GNOME Shell 46–50 | Optional — for the Quick Settings extension |

## Windows

The GUI also runs on Windows, where it supports the subset of features that have
a real Windows path:

| Feature | Windows support |
|---------|-----------------|
| AMD PPT tuning (STAPM / fast / slow) | ✅ via `ryzenadj.exe` |
| Active core count | ✅ via `bcdedit /set {current} numproc` — **requires a reboot** |
| Platform profiles (power plans) | ✅ via [`atrofac`](https://github.com/cronosun/atrofac) (Quiet → silent, Balanced → windows, Performance → turbo) |
| Fan curves | ✅ via `atrofac-cli` — applying a curve also sets the power plan |
| CPU boost / SMT toggles, temperatures | ❌ no sysfs on Windows — hidden in the UI |
| Daemon, GNOME extension | ❌ Linux-only |

### Setup

1. Build the GUI: `cargo build --release --bin strixctl` (do **not** pass
   `--features daemon` — the D-Bus daemon is Linux-only).
2. Obtain [`ryzenadj`](https://github.com/FlyGoat/RyzenAdj) for Windows and place
   `ryzenadj.exe` together with its **WinRing0 driver** (`WinRing0x64.dll` and
   `WinRing0x64.sys`) **next to `strixctl.exe`**. strixctl resolves ryzenadj
   relative to its own executable (override with the `STRIXCTL_RYZENADJ`
   environment variable). The WinRing0 driver has redistribution restrictions, so
   it is not bundled in this repo — download it yourself.
3. For platform profiles and fan curves, obtain
   [`atrofac`](https://github.com/cronosun/atrofac) and place `atrofac-cli.exe`
   next to `strixctl.exe` (override with `STRIXCTL_ATROFAC`). atrofac uses the
   ASUS Armoury Crate WMI interface (no extra driver) and requires Administrator.
4. Run `strixctl.exe`. It starts unprivileged; each PPT, core-count, profile, or
   fan-curve change raises a **UAC prompt** (ryzenadj, bcdedit, and atrofac all
   require Administrator).

### Notes

- Core-count changes use `bcdedit numproc`, which only takes effect after a
  **reboot**. The UI shows a "restart to apply" banner until you reboot.
- atrofac's `fan` command always sets a power plan alongside the curve, so
  applying a fan curve also applies the mapped plan. atrofac is set-only, so the
  current plan/curve can't be read back into the UI.
- Applying a saved profile chains several elevated tools (atrofac plan + fan,
  ryzenadj, bcdedit), each with its own UAC prompt.
- Profiles are stored in `%APPDATA%\strixctl\profiles.json`.
- Reading current PPT values (`ryzenadj --info`) also needs elevation, so it only
  happens when you click **Reload**, never automatically at startup.

## Architecture

```
strixctl (GUI)          strixctld (daemon)
     │                        │
     ├─ asusctl               ├─ com.strixctl.Service  (D-Bus)
     └─ pkexec ryzenadj       │       ├─ ListSavedProfiles
                              │       ├─ ApplySavedProfile
                              │       ├─ SetAsusProfile
                              │       ├─ ApplyPpt
                              │       └─ CurrentTempC (property)
                              │
                    gnome-extension
                        └─ Quick Settings toggle
```

The GUI and daemon are independent — you can run either without the other. The daemon is required only for the GNOME extension or any D-Bus client that wants to trigger profile switches.

## Installation

### 1. Install polkit policy (system-wide, requires sudo)

```sh
make install-polkit
```

This installs `com.strixctl.ryzenadj.policy`, which allows `ryzenadj` to run via `pkexec` without an interactive password prompt from the daemon or a non-active session.

### 2. Install the daemon

```sh
make install-daemon
```

Builds `strixctld` with `cargo install` and writes the D-Bus session activation file so the bus auto-starts the daemon on first use.

### 3. Install the systemd user service (optional)

```sh
make install-systemd
systemctl --user enable --now strixctld
```

### 4. Install the GNOME Shell extension (optional)

```sh
make install-extension
gnome-extensions enable strixctl@strixctl
```

Then reload GNOME Shell: `Alt+F2 → r` (X11) or log out and back in (Wayland).

### All-in-one

```sh
make all
```

Runs all four steps above in order.

### Uninstall

```sh
make uninstall
```

## Building from source

```sh
# GUI only
cargo build --release

# GUI + daemon
cargo build --release --features daemon --bin strixctld
```

## D-Bus interface

Bus name: `com.strixctl.Service`  
Object path: `/com/strixctl/Service`

| Member | Type | Description |
|--------|------|-------------|
| `ListSavedProfiles` | method → `as` | Names of all saved profiles |
| `ApplySavedProfile(name: s)` | method | Apply fan curve then PPT (800 ms apart) |
| `SetAsusProfile(profile: s)` | method | `quiet` / `balanced` / `performance` |
| `ApplyPpt(apu, fast, slow: u)` | method | Set PPT limits in mW directly |
| `CurrentTempC` | property `d` | Live CPU package temperature |
| `TempChanged(temp: d)` | signal | Emitted when temp shifts by > 0.5 °C |

## GNOME extension keybinding

The default shortcut to cycle through saved profiles is `Super+P`. To change it:

```sh
gsettings set org.gnome.shell.extensions.strixctl cycle-profiles-key "['XF86Launch1']"
```

On the ProArt PX13, `Fn+Q` typically reports as `XF86Launch1`.

## Profile storage

Profiles are saved to `~/.config/strixctl/profiles.json`. Each profile can hold PPT limits, a fan curve, or both. The daemon reads this file on every `ApplySavedProfile` call so edits made in the GUI are immediately visible without restarting the daemon.

## License

GPL-2.0-or-later
