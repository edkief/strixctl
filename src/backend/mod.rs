//! Backend: OS-specific hardware and power control.
//!
//! The public API is identical across platforms so the GUI and daemon call the
//! same `backend::*` functions everywhere. Linux implements everything; Windows
//! implements PPT tuning (ryzenadj.exe) and active-core-count control (bcdedit)
//! and provides harmless no-op stubs for controls that have no Windows
//! equivalent (asusctl platform profiles & fan curves, sysfs boost/SMT,
//! temperatures). The GUI hides those controls via `crate::platform` flags, so
//! the stubs are never reached through the UI.

use std::path::PathBuf;

use crate::state::PptLimits;

#[cfg(unix)]
mod linux;
#[cfg(unix)]
pub use linux::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

/// Resolves the ryzenadj executable to invoke.
///
/// Priority:
/// 1. `STRIXCTL_RYZENADJ` environment variable (absolute path or name).
/// 2. On Windows, `ryzenadj.exe` sitting next to the strixctl executable (the
///    bundled copy, shipped alongside its WinRing0 driver files).
/// 3. The bare name on `PATH` (`ryzenadj` / `ryzenadj.exe`).
///
/// On Linux the bare name `ryzenadj` is kept so the existing polkit policy
/// (keyed to `/usr/bin/ryzenadj`) still matches under pkexec.
pub(crate) fn ryzenadj_path() -> PathBuf {
    if let Ok(p) = std::env::var("STRIXCTL_RYZENADJ") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }

    #[cfg(windows)]
    {
        let exe = "ryzenadj.exe";
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let candidate = dir.join(exe);
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from(exe)
    }

    #[cfg(not(windows))]
    {
        PathBuf::from("ryzenadj")
    }
}

/// Parses `ryzenadj --info` table output into PPT limits. The output format is
/// identical on Windows and Linux. ryzenadj reports Watts; we convert to mW.
pub(crate) fn parse_ryzenadj_info(stdout: &str) -> Option<PptLimits> {
    let mut apu: Option<u32> = None;
    let mut fast: Option<u32> = None;
    let mut slow: Option<u32> = None;
    for line in stdout.lines() {
        let lower = line.to_lowercase();
        if lower.contains("stapm limit") {
            apu = parse_ryzenadj_value(line);
        } else if lower.contains("ppt limit fast") {
            fast = parse_ryzenadj_value(line);
        } else if lower.contains("ppt limit slow") {
            slow = parse_ryzenadj_value(line);
        }
    }
    Some(PptLimits {
        apu_limit: apu?,
        fast_limit: fast?,
        slow_limit: slow?,
    })
}

/// Extracts the numeric value (Watts) from a ryzenadj --info table row and
/// converts to mW. Row format: `|STAPM LIMIT            |     15.000|  stapm-limit     |`
fn parse_ryzenadj_value(line: &str) -> Option<u32> {
    let col = line.split('|').nth(2)?;
    let watts: f32 = col.trim().parse().ok()?;
    Some((watts * 1000.0).round() as u32)
}
