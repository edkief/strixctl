//! Compile-time platform capability flags.
//!
//! The GUI branches on these consts so the same view code can hide controls that
//! the running OS has no way to support. Backend functions for unsupported
//! features still exist on every platform (as no-op stubs on Windows), so only
//! the *UI* needs these flags — the call sites stay platform-agnostic.

/// Platform power profiles — asusctl (Quiet / Balanced / Performance) on Linux,
/// atrofac power plans (silent / windows / turbo) on Windows.
pub const SUPPORTS_PLATFORM_PROFILE: bool = true;
/// Fan-curve editor — asusctl on Linux, atrofac-cli on Windows.
pub const SUPPORTS_FAN_CURVE: bool = true;
/// Per-curve hysteresis buffer. asusctl-only; atrofac has no hysteresis argument.
pub const SUPPORTS_FAN_HYSTERESIS: bool = cfg!(unix);
/// CPU boost toggle via sysfs. Linux only.
pub const SUPPORTS_BOOST: bool = cfg!(unix);
/// SMT (sibling-thread) toggle via sysfs. Linux only.
pub const SUPPORTS_SMT: bool = cfg!(unix);
/// CPU/GPU temperature monitoring via sysfs hwmon/thermal. Linux only.
pub const SUPPORTS_TEMP: bool = cfg!(unix);

/// AMD PPT tuning via ryzenadj — available on both Linux and Windows.
pub const SUPPORTS_PPT: bool = true;
/// Active-core-count control — sysfs hotplug on Linux, `bcdedit numproc` on Windows.
pub const SUPPORTS_CORE_PRESET: bool = true;

/// Windows applies core-count changes via `bcdedit numproc`, which only takes
/// effect after a reboot; Linux toggles cpuN/online live. Drives the reboot banner.
pub const CORE_PRESET_NEEDS_REBOOT: bool = cfg!(windows);

/// Reading current PPT (`ryzenadj --info`) needs elevation. On Linux pkexec is
/// silent (polkit policy), so we can read at startup; on Windows it raises a UAC
/// prompt, so we only read on explicit user action (Reload), never automatically.
pub const AUTO_READ_PPT: bool = cfg!(unix);
