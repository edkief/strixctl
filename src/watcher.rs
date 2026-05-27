use std::fs;

pub struct SensorReading {
    pub temp_c: f32,
    pub gpu_temp_c: Option<f32>,
}

pub fn read_now() -> SensorReading {
    SensorReading {
        temp_c: read_cpu_temp().unwrap_or(0.0),
        gpu_temp_c: read_gpu_edge_temp(),
    }
}

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

fn parse_temp_file(path: &str) -> Option<f32> {
    let raw = fs::read_to_string(path).ok()?;
    let millideg: f32 = raw.trim().parse().ok()?;
    Some(millideg / 1000.0)
}
