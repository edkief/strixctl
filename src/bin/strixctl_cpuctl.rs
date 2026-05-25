//! strixctl-cpuctl — privileged helper for CPU boost and core-count control.
//!
//! Called by the main binary / daemon via `pkexec`, guarded by the polkit action
//! com.strixctl.cpuctl (polkit/com.strixctl.cpuctl.policy).
//!
//! Usage:
//!   strixctl-cpuctl boost <0|1>
//!   strixctl-cpuctl cores <4|8|12|16>
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

use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: strixctl-cpuctl boost <0|1>");
        eprintln!("       strixctl-cpuctl cores <4|8|12|16>");
        process::exit(1);
    }

    let result = match args[1].as_str() {
        "boost" => cmd_boost(&args[2]),
        "cores" => cmd_cores(&args[2]),
        other   => Err(format!("unknown command '{other}'")),
    };

    if let Err(e) = result {
        eprintln!("strixctl-cpuctl: {e}");
        process::exit(1);
    }
}

fn cmd_boost(value: &str) -> Result<(), String> {
    let v = match value {
        "0" => "0",
        "1" => "1",
        _ => return Err(format!("boost: expected 0 or 1, got '{value}'")),
    };
    write_sysfs("/sys/devices/system/cpu/cpufreq/boost", v)
}

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

    let mut errors: Vec<String> = Vec::new();
    for i in 0..32u32 {
        if i == 0 {
            // cpu0 cannot be offlined; skip silently.
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

fn write_sysfs(path: &str, value: &str) -> Result<(), String> {
    fs::write(path, value).map_err(|e| format!("write {path}: {e}"))
}
