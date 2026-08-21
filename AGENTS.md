# AGENTS.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository. `CLAUDE.md` is a symlink to it.

## What this is

`strixctl` — power management GUI + session D-Bus daemon for the ASUS ProArt PX13 (AMD Strix Halo) on Linux, with a reduced Windows build. Rust 2024 edition, **iced 0.13** (the README's "egui" mention is stale). GPL-2.0-or-later.

Three binaries from one crate:

| Binary | Path | Notes |
|---|---|---|
| `strixctl` (default-run) | [src/main.rs](src/main.rs) | iced GUI |
| `strixctld` | [src/bin/strixctld.rs](src/bin/strixctld.rs) | D-Bus daemon, `required-features = ["daemon"]` |
| `strixctl-cpuctl` | [src/bin/strixctl_cpuctl.rs](src/bin/strixctl_cpuctl.rs) | privileged sysfs helper run via `pkexec` |

## Commands

```sh
make build                       # all three binaries (release)
cargo build --release            # GUI only
cargo build --release --features daemon --bin strixctld
make build-daemon PPT_DRIFT_GUARD=1   # daemon with the opt-in PPT drift guard

cargo test                       # unit tests (all live in src/state.rs)
cargo test fan_curve_shift_adds_delta   # single test
cargo check                      # fast type check

make all                         # build + install polkit, daemon, systemd unit, extension, bin, desktop
make install-polkit              # sudo; installs both .policy files
make uninstall
```

There is no test harness beyond `cargo test`; no clippy/fmt config is committed.

Running the daemon by hand while iterating:
```sh
systemctl --user restart strixctld && journalctl --user -fu strixctld
```
Daemon diagnostics go to stderr with a `[strixctld]` prefix; the GUI backend logs `[strixctl]`.

## Architecture

### Layering
`views/` + `widgets/` (iced UI) → `app.rs` (Elm-style `Message`/`update`/`view`) → `backend/` (process spawning, sysfs) → external tools. `state.rs` holds the model; `profiles.rs` handles JSON persistence; `watcher.rs` reads sensors.

- [src/app.rs](src/app.rs) is the controller: one `Message` enum, one `update`. Every hardware action is a `spawn_blocking(...)` returning a `*Applied(Result<..., String>)` message — blocking subprocess calls must never run on the UI thread. A 1 s `Tick` subscription refreshes sensors and force-switches to Performance above 95 °C.
- [src/backend/mod.rs](src/backend/mod.rs) defines a platform-identical API; `linux.rs` and `windows.rs` are `cfg`-selected re-exports. Windows keeps no-op stubs for unsupported features so call sites stay platform-agnostic.
- [src/platform.rs](src/platform.rs) holds compile-time `SUPPORTS_*` consts. **Only the UI branches on them** — hide controls there, don't add `cfg` to call sites.
- [src/watcher.rs](src/watcher.rs) scans `/sys/class/hwmon/hwmon{0..20}` matching the `name` file (`k10temp`, `amdgpu`, `asus`) and the `*_label` files, never fixed indices.

### The daemon includes source via `#[path]`
`strixctld.rs` re-declares `state`, `backend`, `profiles` with `#[path = "../state.rs"]` rather than depending on the lib (the crate has no lib target). Any module those three files pull in must stay reachable from that flat inclusion — adding a `use crate::foo` to `backend/linux.rs` breaks the daemon build even when the GUI builds fine. Always build both after touching shared modules.

### Privilege model (Linux)
The GUI never runs as root. Escalation goes through two polkit actions in [polkit/](polkit/):
- `com.strixctl.ryzenadj` → `pkexec ryzenadj` for PPT. The backend deliberately invokes the **bare name `ryzenadj`** on Linux so the policy keyed to `/usr/bin/ryzenadj` still matches. `STRIXCTL_RYZENADJ` overrides the path (mainly for Windows).
- `com.strixctl.cpuctl` → `pkexec /usr/local/bin/strixctl-cpuctl` for boost / SMT / core-count sysfs writes. That path is **hardcoded** in [src/backend/linux.rs](src/backend/linux.rs); `make install-bin` with a non-default `PREFIX` will break it.

`asusctl` (platform profiles, fan curves) needs no escalation — it talks to `asusd`.

### SMU mailbox hazard — read before touching PPT paths
On Strix Halo, `ryzenadj --info` pokes the SMU mailbox that `amdgpu` also uses; a collision **hard-locks the machine**. Everything below exists because of this:

- `--features ppt-drift-guard` is **off by default** (`PPT_DRIFT_GUARD=1` in the Makefile turns it on). It is the only code path that polls `ryzenadj --info` periodically.
- The daemon delays its first SMU-touching work by 30 s after start, and holds off for `RESUME_QUIET_PERIOD` (30 s) after logind's `PrepareForSleep(false)`.
- The platform-profile drift guard (15 s poll) is SMU-safe — it only asks `asusd` over D-Bus — so it always runs.
- Poll loops offset their first tick by a full period rather than firing immediately.

Do not add unconditional `ryzenadj --info` reads. On Linux `platform::AUTO_READ_PPT` allows a startup read; on Windows it is off because the read raises UAC.

### Apply ordering
`asusctl fan-curve --enable-fan-curves true` makes `asusd` reload power state and clobber PPT registers, so the fan curve goes first and PPT waits for asusd to settle. SMT goes before the core preset so the sibling-thread sysfs entries exist (or don't) when `cores` writes to them.

Order in `apply_full_profile` ([src/app.rs](src/app.rs#L749), the `cfg(not(windows))` one):
platform profile → fan curve → **sleep 800 ms** → PPT → boost → SMT → core preset.

`run_apply_saved_profile` ([src/bin/strixctld.rs](src/bin/strixctld.rs#L233)) duplicates this **minus the SMT step** — a D-Bus/GNOME apply leaves SMT untouched where a GUI apply sets it. Known divergence; keep the rest in sync and decide deliberately if you touch either.

After a GUI apply the app tells the daemon via `gdbus call … NotifyProfileApplied` so the GNOME toggle reflects the new active profile.

Unit conventions: PPT in **mW** internally, `ryzenadj --info` reports **W** (`parse_ryzenadj_value` converts). Fan curve points are `(temp °C, speed %)` in the model, converted to asusctl's `NNc:0-255` form on apply. `PptLimits::is_valid` only enforces `slow <= fast` — STAPM is independent.

### D-Bus surface
Bus `com.strixctl.Service`, path `/com/strixctl/Service`, **session bus** (activation file written by `make install-daemon` into `~/.local/share/dbus-1/services/`). Methods: `ListSavedProfiles`, `ApplySavedProfile`, `SetAsusProfile`, `ApplyPpt`, `SetBoost`, `SetCorePreset`, `NotifyProfileApplied`. Properties: `CurrentTempC`, `CurrentPlatformProfile`, `CurrentSavedProfile`, `BoostEnabled`, `ActiveCorePreset`. Signals: `TempChanged` (>0.5 °C delta), `PlatformProfileChanged`, `SavedProfileChanged`, `BoostChanged`, `CorePresetChanged`.

The interface XML is duplicated in [gnome-extension/extension.js](gnome-extension/extension.js#L16) — changing a member means editing both, then `make install-extension` and reloading GNOME Shell.

### Persistence
`~/.config/strixctl/profiles.json` (`%APPDATA%\strixctl` on Windows) plus an `active-profile` file naming the last applied one. The daemon re-reads the JSON on **every** `ApplySavedProfile`, so GUI edits need no daemon restart. New `SavedProfile` fields must carry `#[serde(default …)]` — old files are loaded with `unwrap_or_default()` and a parse failure silently wipes the list.

### Core presets
`CorePreset` is `4|8|12|16` physical cores. Linux toggles `cpuN/online` (topology assumed: 16 cores / 32 threads, cpu0–15 = thread 0, cpu16–31 = SMT siblings, CCD split at 8); Windows uses `bcdedit numproc`, which needs a reboot — hence `CORE_PRESET_NEEDS_REBOOT` and the UI banner.

## Windows build
Never pass `--features daemon` on Windows (zbus is `cfg(unix)`-gated). `ryzenadj.exe` + WinRing0 driver and `atrofac-cli.exe` are resolved next to `strixctl.exe` by `resolve_tool`, overridable via `STRIXCTL_RYZENADJ` / `STRIXCTL_ATROFAC`. Each individual control raises its own UAC prompt, but applying a whole saved profile goes through `backend::apply_saved`, which writes one elevated `.bat` (plan + fan + PPT + `bcdedit`) for a single prompt, with a distinct exit code per step. Boost and SMT are skipped there — no Windows equivalent.

## Not part of the build
[libinput-config/](libinput-config/) is a vendored third-party C project (meson, its own git dir), untracked and unrelated to the Rust build. [specs/overview.md](specs/overview.md) is the original design sketch — historical, not current truth.
