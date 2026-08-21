#[cfg(unix)]
use std::fs;

pub struct SensorReading {
    pub temp_c: f32,
    pub gpu_temp_c: Option<f32>,
    pub cpu_fan_rpm: Option<u32>,
    pub gpu_fan_rpm: Option<u32>,
    /// Battery power draw in watts while discharging; `None` when on AC or when
    /// no battery power sensor is available.
    pub battery_discharge_w: Option<f32>,
    /// Highest live core frequency in MHz. Cores clock independently, so the
    /// maximum across them is the closest single number to "current CPU speed".
    pub cpu_freq_mhz: Option<u32>,
    /// Estimated minutes of battery left at the current draw; `None` on AC or
    /// when the battery exposes no usable energy/power pair.
    pub battery_minutes_left: Option<u32>,
}

pub fn read_now() -> SensorReading {
    #[cfg(unix)]
    {
        let (cpu_fan_rpm, gpu_fan_rpm) = read_fan_rpms();
        SensorReading {
            temp_c: read_cpu_temp().unwrap_or(0.0),
            gpu_temp_c: read_gpu_edge_temp(),
            cpu_fan_rpm,
            gpu_fan_rpm,
            battery_discharge_w: read_battery_discharge_w(),
            cpu_freq_mhz: read_cpu_freq_mhz(),
            battery_minutes_left: read_battery_minutes_left(),
        }
    }

    // No sysfs sensor source on non-Linux platforms; the GUI hides this UI
    // there (see `platform::SUPPORTS_TEMP`).
    #[cfg(not(unix))]
    {
        SensorReading {
            temp_c: 0.0,
            gpu_temp_c: None,
            cpu_fan_rpm: None,
            gpu_fan_rpm: None,
            battery_discharge_w: None,
            cpu_freq_mhz: None,
            battery_minutes_left: None,
        }
    }
}

/// Highest `scaling_cur_freq` across all cpufreq policies, in MHz. Falls back to
/// the largest "cpu MHz" line in /proc/cpuinfo on kernels without cpufreq sysfs.
#[cfg(unix)]
fn read_cpu_freq_mhz() -> Option<u32> {
    let mut best_khz: u32 = 0;
    if let Ok(dir) = fs::read_dir("/sys/devices/system/cpu/cpufreq") {
        for entry in dir.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with("policy") {
                continue;
            }
            // Offline policies keep the file but reading it can fail; skip those.
            if let Some(khz) = fs::read_to_string(entry.path().join("scaling_cur_freq"))
                .ok()
                .and_then(|s| s.trim().parse::<u32>().ok())
            {
                best_khz = best_khz.max(khz);
            }
        }
    }
    if best_khz > 0 {
        return Some((best_khz as f32 / 1000.0).round() as u32);
    }

    let cpuinfo = fs::read_to_string("/proc/cpuinfo").ok()?;
    let best_mhz = cpuinfo
        .lines()
        .filter(|l| l.to_lowercase().starts_with("cpu mhz"))
        .filter_map(|l| l.split(':').nth(1)?.trim().parse::<f32>().ok())
        .fold(0.0f32, f32::max);
    (best_mhz > 0.0).then(|| best_mhz.round() as u32)
}

/// Minutes of battery left at the current discharge rate, from `energy_now`
/// (µWh) / `power_now` (µW), or the charge/current equivalent. Returns `None`
/// unless a battery is actually discharging.
#[cfg(unix)]
fn read_battery_minutes_left() -> Option<u32> {
    let dir = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let base = entry.path();
        let read = |f: &str| fs::read_to_string(base.join(f)).ok();
        let parse = |s: Option<String>| s.and_then(|v| v.trim().parse::<f32>().ok());

        if read("type").map(|t| t.trim().to_string()).as_deref() != Some("Battery") {
            continue;
        }
        if read("status").map(|s| s.trim().to_string()).as_deref() != Some("Discharging") {
            continue;
        }

        // energy/power (Wh, W) and charge/current (Ah, A) both give hours.
        let hours = match (parse(read("energy_now")), parse(read("power_now"))) {
            (Some(e), Some(p)) if p > 0.0 => e / p,
            _ => match (parse(read("charge_now")), parse(read("current_now"))) {
                (Some(c), Some(i)) if i > 0.0 => c / i,
                _ => continue,
            },
        };
        return Some((hours * 60.0).round().clamp(0.0, 6000.0) as u32);
    }
    None
}

#[cfg(unix)]
fn read_cpu_temp() -> Option<f32> {
    // Prefer k10temp Tctl (hwmon), which is the AMD CPU die temperature
    for hwmon in 0..20u8 {
        let base = format!("/sys/class/hwmon/hwmon{hwmon}");
        let name = fs::read_to_string(format!("{base}/name")).unwrap_or_default();
        if name.trim() == "k10temp" {
            if let Some(t) = parse_temp_file(&format!("{base}/temp1_input")) {
                return Some(t);
            }
        }
    }
    // Fallback: thermal_zone labelled x86_pkg_temp or cpu
    for zone in 0..20u8 {
        let base = format!("/sys/class/thermal/thermal_zone{zone}");
        let zone_type = fs::read_to_string(format!("{base}/type")).unwrap_or_default();
        let is_cpu = zone_type.trim() == "x86_pkg_temp" || zone_type.trim().contains("cpu");
        if is_cpu {
            if let Some(t) = parse_temp_file(&format!("{base}/temp")) {
                return Some(t);
            }
        }
    }
    parse_temp_file("/sys/class/thermal/thermal_zone0/temp")
}

#[cfg(unix)]
fn read_gpu_edge_temp() -> Option<f32> {
    for hwmon in 0..20u8 {
        let base = format!("/sys/class/hwmon/hwmon{hwmon}");
        let name = fs::read_to_string(format!("{base}/name")).unwrap_or_default();
        if name.trim() != "amdgpu" {
            continue;
        }
        for idx in 1..=10u8 {
            let label = fs::read_to_string(format!("{base}/temp{idx}_label")).unwrap_or_default();
            if label.trim() == "edge" {
                return parse_temp_file(&format!("{base}/temp{idx}_input"));
            }
        }
    }
    None
}

#[cfg(unix)]
fn parse_temp_file(path: &str) -> Option<f32> {
    let raw = fs::read_to_string(path).ok()?;
    let millideg: f32 = raw.trim().parse().ok()?;
    Some(millideg / 1000.0)
}

/// Reads CPU and GPU fan speeds (RPM) from the `asus` hwmon, matching the
/// `cpu_fan` / `gpu_fan` labels rather than fixed indices. The asus-wmi fan
/// channels report 0 RPM when the fans are idle, which we surface as `Some(0)`.
#[cfg(unix)]
fn read_fan_rpms() -> (Option<u32>, Option<u32>) {
    for hwmon in 0..20u8 {
        let base = format!("/sys/class/hwmon/hwmon{hwmon}");
        let name = fs::read_to_string(format!("{base}/name")).unwrap_or_default();
        if name.trim() != "asus" {
            continue;
        }
        let mut cpu = None;
        let mut gpu = None;
        for idx in 1..=8u8 {
            let label = fs::read_to_string(format!("{base}/fan{idx}_label")).unwrap_or_default();
            let rpm = || {
                fs::read_to_string(format!("{base}/fan{idx}_input"))
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok())
            };
            match label.trim() {
                "cpu_fan" => cpu = rpm(),
                "gpu_fan" => gpu = rpm(),
                _ => {}
            }
        }
        return (cpu, gpu);
    }
    (None, None)
}

/// Reads battery power draw in watts, returning `Some` only while discharging.
/// Prefers `power_now` (µW); falls back to `current_now` × `voltage_now` for
/// batteries that expose only charge-based sensors.
#[cfg(unix)]
fn read_battery_discharge_w() -> Option<f32> {
    let dir = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in dir.flatten() {
        let base = entry.path();
        let read = |f: &str| fs::read_to_string(base.join(f)).ok();

        if read("type").map(|t| t.trim().to_string()).as_deref() != Some("Battery") {
            continue;
        }
        if read("status").map(|s| s.trim().to_string()).as_deref() != Some("Discharging") {
            continue;
        }

        let parse = |s: Option<String>| s.and_then(|v| v.trim().parse::<f32>().ok());

        if let Some(uw) = parse(read("power_now")) {
            return Some(uw / 1_000_000.0);
        }
        if let (Some(ua), Some(uv)) = (parse(read("current_now")), parse(read("voltage_now"))) {
            return Some((ua / 1_000_000.0) * (uv / 1_000_000.0));
        }
    }
    None
}
