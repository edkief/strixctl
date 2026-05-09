use std::process::Command;

use crate::state::{FanCurve, PptLimits, Profile};

pub fn apply_profile(profile: &Profile) -> Result<(), String> {
    run_cmd("asusctl", &["profile", "set", profile.as_str()])
}

pub fn apply_ppt(ppt: &PptLimits) -> Result<(), String> {
    run_cmd("pkexec", &[
        "ryzenadj",
        &format!("--stapm-limit={}", ppt.apu_limit),
        &format!("--fast-limit={}", ppt.fast_limit),
        &format!("--slow-limit={}", ppt.slow_limit),
        &format!("--apu-slow-limit={}", ppt.apu_limit),
    ])
}

/// Speed values in FanCurve are stored as percentages (0.0–100.0) mapped from
/// asusctl's PWM range (0–255). We write them back as raw PWM (no '%') to
/// avoid precision loss and to satisfy asusctl's strict 8-point requirement.
pub fn apply_fan_curve(profile: &Profile, curve: &FanCurve) -> Result<(), String> {
    let points_str = curve
        .points
        .iter()
        .map(|(t, s)| format!("{}c:{}", *t as u8, (*s / 100.0 * 255.0).round() as u8))
        .collect::<Vec<_>>()
        .join(",");
    eprintln!("[strixctl] apply_fan_curve: profile={} points={}", profile.as_str(), points_str);
    let profile_str = profile.as_str();
    for fan in ["cpu", "gpu"] {
        run_cmd("asusctl", &[
            "fan-curve",
            "--mod-profile", profile_str,
            "--fan", fan,
            "--data", &points_str,
        ])?;
    }
    run_cmd("asusctl", &[
        "fan-curve",
        "--mod-profile", profile_str,
        "--enable-fan-curves", "true",
    ])
}

/// Reads the CPU fan curve for `profile` from asusctl.
pub fn read_fan_curve(profile: &Profile) -> Option<FanCurve> {
    let out = Command::new("asusctl")
        .args(["fan-curve", "--mod-profile", profile.as_str()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_cpu_fan_curve(&String::from_utf8_lossy(&out.stdout))
}

/// Parses `asusctl fan-curve --mod-profile` output, e.g.:
///   (fan: CPU, pwm: (1, 15, 30, ...), temp: (53, 54, 59, ...), enabled: true)
/// PWM values (0–255) are normalised to percentages for the FanCurve.
fn parse_cpu_fan_curve(output: &str) -> Option<FanCurve> {
    let lines: Vec<&str> = output.lines().collect();
    let cpu_idx = lines.iter().position(|l| {
        let t = l.trim().to_lowercase();
        t.starts_with("fan:") && t.contains("cpu")
    })?;

    let window_end = (cpu_idx + 8).min(lines.len());
    let mut temps: Option<Vec<u32>> = None;
    let mut pwms: Option<Vec<u32>> = None;

    for line in &lines[cpu_idx..window_end] {
        let t = line.trim();
        let lower = t.to_lowercase();
        if lower.starts_with("temp:") {
            temps = extract_paren_u32s(t);
        } else if lower.starts_with("pwm:") {
            pwms = extract_paren_u32s(t);
        }
    }

    let temps = temps?;
    let pwms = pwms?;
    if temps.len() != pwms.len() || temps.is_empty() {
        return None;
    }

    let points = temps.iter().zip(pwms.iter())
        .map(|(&t, &p)| (t as f32, p as f32 / 255.0 * 100.0))
        .collect();
    Some(FanCurve { points, hysteresis: 2 })
}

/// Extracts `u32` values from a `(a, b, c, ...)` tuple on a single line.
fn extract_paren_u32s(s: &str) -> Option<Vec<u32>> {
    let start = s.find('(')?;
    let end = s.rfind(')')?;
    if end <= start { return None; }
    s[start + 1..end]
        .split(',')
        .map(|v| v.trim().parse::<u32>().ok())
        .collect()
}

/// Reads current PPT limits from `ryzenadj --info` (requires pkexec).
/// ryzenadj reports values in Watts; we convert to mW for storage.
pub fn read_current_ppt() -> Option<PptLimits> {
    let out = Command::new("pkexec")
        .args(["ryzenadj", "--info"])
        .output()
        .ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
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

/// Extracts the numeric value (Watts) from a ryzenadj --info table row and converts to mW.
/// Row format: `|STAPM LIMIT            |     15.000|  stapm-limit     |`
fn parse_ryzenadj_value(line: &str) -> Option<u32> {
    let col = line.split('|').nth(2)?;
    let watts: f32 = col.trim().parse().ok()?;
    Some((watts * 1000.0).round() as u32)
}

pub fn read_current_profile() -> Option<Profile> {
    let out = Command::new("asusctl").arg("profile").arg("get").output().ok()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    // Parse "Active profile: <name>" line
    let active = stdout
        .lines()
        .find(|l| l.to_lowercase().starts_with("active profile:"))?
        .split(':')
        .nth(1)?
        .trim()
        .to_lowercase();
    match active.as_str() {
        "quiet" => Some(Profile::Quiet),
        "performance" => Some(Profile::Performance),
        _ => Some(Profile::Balanced),
    }
}

fn run_cmd(prog: &str, args: &[&str]) -> Result<(), String> {
    eprintln!("[strixctl] >> {} {}", prog, args.join(" "));
    let out = Command::new(prog)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {prog}: {e}"))?;

    // Print all output lines uniformly — ryzenadj emits informational text on
    // stderr, so we don't bother distinguishing the two streams here.
    for line in String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(String::from_utf8_lossy(&out.stderr).lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
        eprintln!("[strixctl]  | {line}");
    }

    if out.status.success() {
        eprintln!("[strixctl] ok");
        Ok(())
    } else {
        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        eprintln!("[strixctl] FAILED ({})", out.status);
        Err(msg)
    }
}
