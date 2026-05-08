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
