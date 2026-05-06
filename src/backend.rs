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
    ])
}

pub fn apply_fan_curve(curve: &FanCurve) -> Result<(), String> {
    let points_str = curve
        .points
        .iter()
        .map(|(t, s)| format!("{}c:{}%", *t as u8, *s as u8))
        .collect::<Vec<_>>()
        .join(",");
    run_cmd("asusctl", &["fan-curve", "-f", &points_str])
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
    let out = Command::new(prog)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {prog}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
