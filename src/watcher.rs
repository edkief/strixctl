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
        }
    }
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
