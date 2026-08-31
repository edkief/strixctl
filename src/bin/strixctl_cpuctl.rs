//! strixctl-cpuctl — privileged helper for CPU boost and core-count control.
//!
//! Called by the main binary / daemon via `pkexec`, guarded by the polkit action
//! com.strixctl.cpuctl (polkit/com.strixctl.cpuctl.policy).
//!
//! Usage:
//!   strixctl-cpuctl boost   <0|1>
//!   strixctl-cpuctl cores   <4|8|12|16>
//!   strixctl-cpuctl smt     <0|1>
//!   strixctl-cpuctl maxfreq <kHz|max>
//!   strixctl-cpuctl online-all now
//!
//! SMT is controlled globally via /sys/devices/system/cpu/smt/control (on|off).
//! When SMT is off, sibling threads (cpu16-31) are absent from sysfs, so
//! `cores` skips them silently.
//!
//! Core topology assumed: 16 physical cores / 32 logical (SMT on), cpu0–31.
//!   cpu0–15  = core 0–15, thread 0   (CCD0: 0–7, CCD1: 8–15)
//!   cpu16–31 = core 0–15, thread 1   (SMT siblings)
//!
//! Presets (SMT stays on; each active physical core keeps both threads):
//!   4  cores (8 threads) : cpu 0–3,  16–19          online; rest offline
//!   8  cores (16 threads): cpu 0–7,  16–23          online; rest offline
//!   12 cores (24 threads): cpu 0–5,  8–13, 16–21, 24–29 online; rest offline
//!   16 cores (32 threads): all cpu 0–31              online
//!
//! cpu0 is always online (kernel ignores writes to /sys/.../cpu0/online).

use std::process;
#[cfg(unix)]
use std::fs;

// This privileged helper drives Linux sysfs directly and is unix-only. On other
// platforms it builds as a stub so `cargo build` over the whole package succeeds;
// it is never installed or invoked there.
#[cfg(not(unix))]
fn main() {
    eprintln!("strixctl-cpuctl is only supported on Linux.");
    process::exit(1);
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: strixctl-cpuctl boost   <0|1>");
        eprintln!("       strixctl-cpuctl cores   <4|8|12|16>");
        eprintln!("       strixctl-cpuctl smt     <0|1>");
        eprintln!("       strixctl-cpuctl maxfreq <kHz|max>");
        eprintln!("       strixctl-cpuctl online-all now");
        process::exit(1);
    }

    let result = match args[1].as_str() {
        "boost"   => cmd_boost(&args[2]),
        "cores"   => cmd_cores(&args[2]),
        "smt"     => cmd_smt(&args[2]),
        "maxfreq" => cmd_maxfreq(&args[2]),
        "online-all" => cmd_online_all(&args[2]),
        other     => Err(format!("unknown command '{other}'")),
    };

    if let Err(e) = result {
        eprintln!("strixctl-cpuctl: {e}");
        process::exit(1);
    }
}

/// Brings every CPU reported by the kernel's `present` mask online. This is
/// deliberately independent of the machine-specific core preset topology:
/// CPUs must not remain offline when firmware enters s2idle.
#[cfg(unix)]
fn cmd_online_all(value: &str) -> Result<(), String> {
    if value != "now" {
        return Err(format!("online-all: expected 'now', got '{value}'"));
    }

    // With SMT administratively off, its sibling CPUs remain in `present` but
    // reject online writes. Temporarily enable SMT so the final present/online
    // equality is meaningful; the daemon restores the active profile on resume.
    if let Ok(control) = fs::read_to_string("/sys/devices/system/cpu/smt/control") {
        if control.trim().eq_ignore_ascii_case("off") {
            write_sysfs("/sys/devices/system/cpu/smt/control", "on")
                .map_err(|e| format!("online-all: enable SMT: {e}"))?;
        }
    }

    let present_raw = fs::read_to_string("/sys/devices/system/cpu/present")
        .map_err(|e| format!("online-all: cannot read present CPUs: {e}"))?;
    let present = parse_cpu_list(&present_raw)?;
    if present.is_empty() {
        return Err("online-all: kernel reported no present CPUs".into());
    }

    let mut write_errors = Vec::new();
    for cpu in present.iter().copied().filter(|cpu| *cpu != 0) {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/online");
        if !std::path::Path::new(&path).exists() {
            write_errors.push(format!("cpu{cpu}: missing {path}"));
        } else if let Err(e) = write_sysfs(&path, "1") {
            write_errors.push(format!("cpu{cpu}: {e}"));
        }
    }

    let online_raw = fs::read_to_string("/sys/devices/system/cpu/online")
        .map_err(|e| format!("online-all: cannot verify online CPUs: {e}"))?;
    let online = parse_cpu_list(&online_raw)?;
    let missing: Vec<u32> = present.difference(&online).copied().collect();
    if missing.is_empty() {
        if !write_errors.is_empty() {
            eprintln!(
                "strixctl-cpuctl: online-all verified despite transient write errors: {}",
                write_errors.join("; ")
            );
        }
        Ok(())
    } else {
        let mut details = Vec::new();
        if !missing.is_empty() {
            details.push(format!("still offline: {}", format_cpu_list(&missing)));
        }
        if !write_errors.is_empty() {
            details.push(format!("write failures: {}", write_errors.join("; ")));
        }
        Err(format!("online-all: {}", details.join("; ")))
    }
}

#[cfg(unix)]
fn parse_cpu_list(raw: &str) -> Result<std::collections::BTreeSet<u32>, String> {
    let mut cpus = std::collections::BTreeSet::new();
    for part in raw.trim().split(',').filter(|part| !part.is_empty()) {
        let (start, end) = match part.split_once('-') {
            Some((start, end)) => (start, end),
            None => (part, part),
        };
        let start: u32 = start.parse()
            .map_err(|_| format!("invalid CPU list '{raw}'"))?;
        let end: u32 = end.parse()
            .map_err(|_| format!("invalid CPU list '{raw}'"))?;
        if start > end {
            return Err(format!("invalid CPU range '{part}'"));
        }
        cpus.extend(start..=end);
    }
    Ok(cpus)
}

#[cfg(unix)]
fn format_cpu_list(cpus: &[u32]) -> String {
    cpus.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
}

#[cfg(unix)]
fn cmd_boost(value: &str) -> Result<(), String> {
    let v = match value {
        "0" => "0",
        "1" => "1",
        _ => return Err(format!("boost: expected 0 or 1, got '{value}'")),
    };
    write_sysfs("/sys/devices/system/cpu/cpufreq/boost", v)
}

#[cfg(unix)]
fn cmd_cores(value: &str) -> Result<(), String> {
    let preset: u32 = value.parse()
        .map_err(|_| format!("cores: expected 4|8|12|16, got '{value}'"))?;

    // online[N] = true means cpuN should be online for this preset.
    // cpu0 is always online; writes to it are skipped.
    let mut online = [false; 32];
    match preset {
        4 => {
            // cpu0-3, cpu16-19
            for i in 0..4   { online[i]    = true; }
            for i in 16..20 { online[i]    = true; }
        }
        8 => {
            // cpu0-7, cpu16-23
            for i in 0..8   { online[i]    = true; }
            for i in 16..24 { online[i]    = true; }
        }
        12 => {
            // CCD0: cpu0-5, cpu16-21  (6 physical + SMT)
            // CCD1: cpu8-13, cpu24-29 (6 physical + SMT)
            for i in 0..6   { online[i]    = true; }
            for i in 8..14  { online[i]    = true; }
            for i in 16..22 { online[i]    = true; }
            for i in 24..30 { online[i]    = true; }
        }
        16 => {
            // All CPUs online
            for i in 0..32 { online[i] = true; }
        }
        _ => return Err(format!("cores: expected 4|8|12|16, got '{value}'")),
    }

    // When SMT is disabled the kernel hides sibling threads (cpu16-31) entirely,
    // so writing to them would error. Skip the sibling range in that case.
    let smt_off = fs::read_to_string("/sys/devices/system/cpu/smt/control")
        .map(|s| s.trim().eq_ignore_ascii_case("off"))
        .unwrap_or(false);

    let mut errors: Vec<String> = Vec::new();
    for i in 0..32u32 {
        if i == 0 {
            // cpu0 cannot be offlined; skip silently.
            continue;
        }
        if smt_off && i >= 16 {
            continue;
        }
        let path = format!("/sys/devices/system/cpu/cpu{i}/online");
        // Skip CPUs that don't have an online file (absent on some kernel configs).
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        let val = if online[i as usize] { "1" } else { "0" };
        if let Err(e) = write_sysfs(&path, val) {
            errors.push(format!("cpu{i}: {e}"));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(unix)]
fn cmd_smt(value: &str) -> Result<(), String> {
    let v = match value {
        "0" => "off",
        "1" => "on",
        _ => return Err(format!("smt: expected 0 or 1, got '{value}'")),
    };
    write_sysfs("/sys/devices/system/cpu/smt/control", v)
}

/// Caps the maximum CPU frequency. `value` is a frequency in kHz, or "max" to
/// restore the hardware maximum.
///
/// Prefers `cpupower frequency-set -u`, which applies the limit to every policy
/// through the kernel's own tooling; falls back to writing `scaling_max_freq`
/// directly when cpupower is not installed (it ships in a separate package on
/// most distributions, and the sysfs write is exactly what it does anyway).
#[cfg(unix)]
fn cmd_maxfreq(value: &str) -> Result<(), String> {
    let khz = if value.eq_ignore_ascii_case("max") {
        // The hardware ceiling, which already reflects the current boost state.
        read_khz("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq")
            .ok_or_else(|| "maxfreq: cannot read cpuinfo_max_freq".to_string())?
    } else {
        let khz: u32 = value
            .parse()
            .map_err(|_| format!("maxfreq: expected a frequency in kHz or 'max', got '{value}'"))?;
        let hw_min = read_khz("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_min_freq");
        let hw_max = read_khz("/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq");
        match (hw_min, hw_max) {
            // Clamp rather than reject: an out-of-range write fails per policy
            // and would leave the CPUs in a half-applied state.
            (Some(lo), Some(hi)) => khz.clamp(lo, hi),
            _ => khz,
        }
    };

    if cpupower_set_max(khz).is_ok() {
        return Ok(());
    }
    write_scaling_max_freq(khz)
}

#[cfg(unix)]
fn read_khz(path: &str) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// `cpupower frequency-set -u <khz>` — bare numbers are kHz. Errors (including
/// "not installed") are returned so the caller can fall back to sysfs.
#[cfg(unix)]
fn cpupower_set_max(khz: u32) -> Result<(), String> {
    let out = process::Command::new("cpupower")
        .args(["frequency-set", "-u", &khz.to_string()])
        .output()
        .map_err(|e| format!("cpupower: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Writes `scaling_max_freq` on every cpufreq policy. Offline policies and
/// policies that reject the value are collected rather than aborting, so one
/// failing CPU doesn't leave the rest untouched.
#[cfg(unix)]
fn write_scaling_max_freq(khz: u32) -> Result<(), String> {
    let dir = fs::read_dir("/sys/devices/system/cpu/cpufreq")
        .map_err(|e| format!("maxfreq: no cpufreq sysfs: {e}"))?;

    let mut errors: Vec<String> = Vec::new();
    let mut written = 0usize;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("policy") {
            continue;
        }
        let path = entry.path().join("scaling_max_freq");
        if !path.exists() {
            continue;
        }
        match fs::write(&path, khz.to_string()) {
            Ok(()) => written += 1,
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }

    // Failures are usually identical across all 32 policies (permissions, or a
    // value the driver rejects), so report a count and one example instead of
    // pasting the same message dozens of times into the GUI status bar.
    if written == 0 {
        return Err(match errors.first() {
            None => "maxfreq: no cpufreq policies found".to_string(),
            Some(first) => format!("maxfreq: all {} policies failed ({first})", errors.len()),
        });
    }
    match errors.first() {
        None => Ok(()),
        Some(first) => Err(format!(
            "maxfreq: applied to {written} policies, {} failed ({first})",
            errors.len()
        )),
    }
}

#[cfg(unix)]
fn write_sysfs(path: &str, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("write {path}: {e}"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::parse_cpu_list;

    #[test]
    fn parses_kernel_cpu_lists() {
        let parsed = parse_cpu_list("0-3,8,10-11\n").unwrap();
        assert_eq!(parsed.into_iter().collect::<Vec<_>>(), vec![0, 1, 2, 3, 8, 10, 11]);
    }

    #[test]
    fn rejects_reversed_cpu_ranges() {
        assert!(parse_cpu_list("4-2").is_err());
    }
}
