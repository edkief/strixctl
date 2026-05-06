use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub struct SensorReading {
    pub temp_c: f32,
}

pub fn spawn_watcher(tx: mpsc::Sender<SensorReading>) {
    thread::spawn(move || loop {
        if let Some(temp_c) = read_cpu_temp() {
            let _ = tx.send(SensorReading { temp_c });
        }
        thread::sleep(Duration::from_secs(1));
    });
}

fn read_cpu_temp() -> Option<f32> {
    // Try each thermal zone; prefer one labelled x86_pkg_temp
    for zone in 0..20u8 {
        let base = format!("/sys/class/thermal/thermal_zone{zone}");
        let type_path = format!("{base}/type");
        let temp_path = format!("{base}/temp");

        let zone_type = fs::read_to_string(&type_path).unwrap_or_default();
        let is_cpu = zone_type.trim() == "x86_pkg_temp" || zone_type.trim().contains("cpu");

        if is_cpu {
            if let Some(t) = parse_temp_file(&temp_path) {
                return Some(t);
            }
        }
    }
    // Fallback: zone 0
    parse_temp_file("/sys/class/thermal/thermal_zone0/temp")
}

fn parse_temp_file(path: &str) -> Option<f32> {
    let raw = fs::read_to_string(path).ok()?;
    let millideg: f32 = raw.trim().parse().ok()?;
    Some(millideg / 1000.0)
}
