//! Windows backend.
//!
//! Implements the features that have a real Windows path:
//!   * AMD PPT tuning via `ryzenadj.exe` (identical CLI to Linux), run elevated.
//!   * Active-core-count control via `bcdedit /set {current} numproc N`, which
//!     only takes effect after a reboot (tracked so the UI can warn the user).
//!   * Max CPU frequency via the `PROCFREQMAX` power-plan setting (`powercfg`),
//!     the Windows equivalent of the Linux cpufreq cap.
//!
//! Everything else (asusctl platform profiles & fan curves, sysfs boost/SMT,
//! temperatures) has no Windows equivalent and is exposed here as a harmless
//! no-op stub so the shared GUI/daemon code compiles unchanged. The GUI hides
//! these controls via `crate::platform`, so the stubs are never reached.
//!
//! Privilege model: there is no pkexec on Windows, so each privileged call
//! elevates on demand via `ShellExecuteExW` with the `runas` verb (one UAC
//! prompt per action). The GUI itself runs unprivileged.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use crate::profiles::SavedProfile;
use crate::state::{CorePreset, FanCurve, PptLimits, Profile};

// ---------- Supported features ----------

pub fn apply_ppt(ppt: &PptLimits) -> Result<(), String> {
    let ryzenadj = super::ryzenadj_path();
    let args = format!(
        "--stapm-limit={} --fast-limit={} --slow-limit={} --apu-slow-limit={}",
        ppt.apu_limit, ppt.fast_limit, ppt.slow_limit, ppt.apu_limit
    );
    run_elevated(ryzenadj.as_os_str(), &args)
}

/// Reads current PPT from `ryzenadj --info`. An elevated process can't write to
/// our pipes, so we run it through `cmd.exe` redirecting stdout to a temp file,
/// then parse the file. This triggers a UAC prompt, so the GUI only calls it on
/// an explicit Reload (never at startup — see `platform::AUTO_READ_PPT`).
pub fn read_current_ppt() -> Option<PptLimits> {
    let ryzenadj = super::ryzenadj_path();
    let tmp = std::env::temp_dir().join("strixctl_ryzenadj_info.txt");
    let _ = std::fs::remove_file(&tmp);

    // cmd.exe quoting: the whole command after /c is wrapped in an extra pair of
    // quotes so cmd keeps the quoted exe path and redirect target intact.
    let cmd = format!(
        "/c \"\"{}\" --info > \"{}\"\"",
        ryzenadj.display(),
        tmp.display()
    );
    run_elevated(OsStr::new("cmd.exe"), &cmd).ok()?;

    let out = std::fs::read_to_string(&tmp).ok()?;
    let _ = std::fs::remove_file(&tmp);
    super::parse_ryzenadj_info(&out)
}

/// Applies a core-count preset via `bcdedit numproc`. Requires a reboot to take
/// effect; the requested preset is persisted so `core_reboot_pending` can report
/// the pending state until the system actually reboots.
pub fn set_core_preset(preset: &CorePreset) -> Result<(), String> {
    let args = match preset_numproc(preset) {
        Some(n) => format!("/set {{current}} numproc {n}"),
        None => "/deletevalue {current} numproc".to_string(),
    };
    run_elevated(OsStr::new("bcdedit.exe"), &args)?;
    write_pending_cores(preset.as_u32());
    Ok(())
}

/// `numproc` caps *logical* processors. Our presets are physical-core counts and
/// Strix Halo runs SMT (2 threads/core), so a preset maps to `cores * 2` logical
/// processors. "Sixteen" means "all cores" → remove the cap entirely.
fn preset_numproc(preset: &CorePreset) -> Option<u32> {
    match preset {
        CorePreset::Sixteen => None,
        other => Some(other.as_u32() * 2),
    }
}

/// Returns the requested preset while a reboot is pending (so the selector
/// reflects the user's intent), otherwise the actually-active count derived from
/// the live logical-processor count.
pub fn read_core_preset() -> CorePreset {
    if let Some(req) = read_pending_cores() {
        return CorePreset::from_u32(req);
    }
    CorePreset::from_u32(active_cores())
}

/// A reboot is pending when the last requested preset differs from what's
/// actually active. Once they converge (after a reboot) the marker is cleared.
pub fn core_reboot_pending() -> bool {
    match read_pending_cores() {
        Some(req) => {
            if req == active_cores() {
                clear_pending_cores();
                false
            } else {
                true
            }
        }
        None => false,
    }
}

/// Physical-core count derived from the live logical-processor count (SMT on).
fn active_cores() -> u32 {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(32);
    (threads / 2).max(1)
}

fn pending_path() -> PathBuf {
    crate::profiles::config_dir().join("pending-cores")
}

fn read_pending_cores() -> Option<u32> {
    std::fs::read_to_string(pending_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn write_pending_cores(n: u32) {
    let path = pending_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, n.to_string());
}

fn clear_pending_cores() {
    let _ = std::fs::remove_file(pending_path());
}

// ---------- Max CPU frequency via powercfg ----------
//
// PROCFREQMAX is the power-plan "Maximum processor frequency" setting, in MHz,
// where 0 means unlimited. It is the closest Windows equivalent to writing
// cpufreq's scaling_max_freq on Linux. Note that the value lives on the active
// power *plan*, so switching plans (which `apply_profile` does through atrofac)
// can change which cap is in force.

/// Applies a max-frequency cap. `None` removes the cap (PROCFREQMAX = 0).
///
/// AC and DC values are both set so the cap holds on battery and on mains, then
/// the scheme is re-activated because powercfg only commits changes on
/// `/setactive`. All three run in one elevated `cmd` line = one UAC prompt.
pub fn set_max_freq_khz(khz: Option<u32>) -> Result<(), String> {
    let mhz = khz.map(|k| (k as f32 / 1000.0).round() as u32).unwrap_or(0);
    let cmd = format!(
        "/c powercfg /setacvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCFREQMAX {mhz} && \
         powercfg /setdcvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCFREQMAX {mhz} && \
         powercfg /setactive SCHEME_CURRENT"
    );
    run_elevated(OsStr::new("cmd.exe"), &cmd)?;
    write_last_max_freq(khz);
    Ok(())
}

/// Returns the last cap this app applied. Querying powercfg for the live value
/// needs elevation and output parsing for no real benefit, so the UI shows what
/// we last set instead (see `platform::MAX_FREQ_READBACK`).
pub fn read_max_freq_khz() -> Option<u32> {
    std::fs::read_to_string(max_freq_path()).ok()?.trim().parse().ok()
}

/// Windows exposes no reliable min/max frequency pair to bound the input with.
pub fn read_freq_range_khz() -> Option<(u32, u32)> {
    None
}

fn max_freq_path() -> PathBuf {
    crate::profiles::config_dir().join("max-freq-khz")
}

fn write_last_max_freq(khz: Option<u32>) {
    let path = max_freq_path();
    match khz {
        Some(k) => {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(path, k.to_string());
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ---------- Power plan & fan curve via atrofac-cli ----------
//
// atrofac drives the ASUS Armoury Crate WMI interface. Its `fan` command sets a
// power plan *and* the fan curve together; `plan` sets the plan only. Both need
// Administrator (no kernel driver) and atrofac cannot read state back.

/// Sets the atrofac power plan (keeping ASUS's default fan curve).
pub fn apply_profile(profile: &Profile) -> Result<(), String> {
    let atrofac = atrofac_path();
    run_elevated(atrofac.as_os_str(), &format!("plan {}", profile_plan(profile)))
}

/// Sets the fan curve (applied to both CPU and GPU fans) together with the plan
/// that maps to `profile`, since atrofac's `fan` command always sets both.
pub fn apply_fan_curve(profile: &Profile, curve: &FanCurve) -> Result<(), String> {
    let atrofac = atrofac_path();
    let c = format_curve(curve)?;
    let args = format!("fan --plan {} --cpu {c} --gpu {c}", profile_plan(profile));
    run_elevated(atrofac.as_os_str(), &args)
}

/// atrofac is set-only — it cannot read the current curve back.
pub fn read_fan_curve(_profile: &Profile) -> Option<FanCurve> {
    None
}

/// atrofac cannot read the current plan back.
pub fn read_current_profile() -> Option<Profile> {
    None
}

fn atrofac_path() -> PathBuf {
    super::resolve_tool("STRIXCTL_ATROFAC", "atrofac-cli.exe", "atrofac-cli")
}

/// Maps a strixctl profile to an atrofac power plan.
fn profile_plan(profile: &Profile) -> &'static str {
    match profile {
        Profile::Quiet => "silent",
        Profile::Balanced => "windows",
        Profile::Performance => "turbo",
    }
}

/// Formats a FanCurve as atrofac's `30c:0%,40c:5%,…` string. atrofac requires
/// exactly 8 points. Speed values are already percentages (0–100), unlike the
/// Linux/asusctl path which writes raw PWM.
fn format_curve(curve: &FanCurve) -> Result<String, String> {
    if curve.points.len() != 8 {
        return Err(format!(
            "atrofac requires exactly 8 fan-curve points, got {}",
            curve.points.len()
        ));
    }
    Ok(curve
        .points
        .iter()
        .map(|(t, s)| format!("{}c:{}%", *t as u8, s.round() as u8))
        .collect::<Vec<_>>()
        .join(","))
}

// ---------- Batched apply (single UAC prompt) ----------

/// Applies a whole saved profile (plan + fan curve + PPT + core preset) in one
/// elevated batch script, so the user sees a single UAC prompt instead of one
/// per tool. Each step gets a distinct exit code so a failure can be named.
///
/// Boost and SMT have no Windows equivalent and are skipped. The core-preset
/// step still only takes effect after a reboot (tracked via `write_pending_cores`).
pub fn apply_saved(saved: &SavedProfile) -> Result<(), String> {
    let atrofac = atrofac_path();
    let ryzenadj = super::ryzenadj_path();

    let curve = format_curve(&FanCurve {
        points: saved.fan_curve.clone(),
        hysteresis: saved.fan_hysteresis,
    })?;
    // In a .bat, `%` starts variable expansion — double it so atrofac receives
    // a literal `%` (cmd collapses `%%` back to `%`).
    let curve = curve.replace('%', "%%");
    let plan = profile_plan(&saved.platform_profile);
    let ppt = &saved.ppt;

    let dir = std::env::temp_dir();
    let bat = dir.join("strixctl_apply.bat");
    let log = dir.join("strixctl_apply.log");
    let _ = std::fs::remove_file(&log);

    // `/set` is fatal on failure; `/deletevalue` ("all cores") is best-effort,
    // since deleting an unset value errors harmlessly.
    let bcd_line = match preset_numproc(&saved.core_preset) {
        Some(n) => format!(
            "bcdedit /set {{current}} numproc {n} >> \"{log}\" 2>&1 || exit /b 3",
            log = log.display()
        ),
        None => format!(
            "bcdedit /deletevalue {{current}} numproc >> \"{log}\" 2>&1",
            log = log.display()
        ),
    };

    // The frequency cap is part of the power plan, so it must be written after
    // atrofac has switched plans — otherwise the plan switch discards it.
    let freq_line = match saved.max_freq_khz {
        Some(khz) => {
            let mhz = (khz as f32 / 1000.0).round() as u32;
            format!(
                "powercfg /setacvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCFREQMAX {mhz} >> \"{log}\" 2>&1 || exit /b 4\r\n\
                 powercfg /setdcvalueindex SCHEME_CURRENT SUB_PROCESSOR PROCFREQMAX {mhz} >> \"{log}\" 2>&1 || exit /b 4\r\n\
                 powercfg /setactive SCHEME_CURRENT >> \"{log}\" 2>&1 || exit /b 4",
                log = log.display(),
            )
        }
        // No cap stored in the profile: leave whatever the plan defines alone.
        None => "rem no frequency cap in profile".to_string(),
    };

    let script = format!(
        "@echo off\r\n\
         \"{atro}\" fan --plan {plan} --cpu {curve} --gpu {curve} >> \"{log}\" 2>&1 || exit /b 1\r\n\
         \"{ryz}\" --stapm-limit={apu} --fast-limit={fast} --slow-limit={slow} --apu-slow-limit={apu} >> \"{log}\" 2>&1 || exit /b 2\r\n\
         {bcd}\r\n\
         {freq}\r\n",
        atro = atrofac.display(),
        ryz = ryzenadj.display(),
        log = log.display(),
        apu = ppt.apu_limit,
        fast = ppt.fast_limit,
        slow = ppt.slow_limit,
        bcd = bcd_line,
        freq = freq_line,
    );
    std::fs::write(&bat, script).map_err(|e| format!("write batch file: {e}"))?;

    let code = run_elevated_code(OsStr::new("cmd.exe"), &format!("/c \"{}\"", bat.display()));
    let _ = std::fs::remove_file(&bat);
    let code = code?; // propagate elevation/launch failure (e.g. UAC declined)

    let result = match code {
        0 => {
            // All steps ran, including bcdedit — the core change is now pending.
            write_pending_cores(saved.core_preset.as_u32());
            if saved.max_freq_khz.is_some() {
                write_last_max_freq(saved.max_freq_khz);
            }
            Ok(())
        }
        1 => Err(step_error("power plan / fan curve (atrofac)", &log)),
        2 => Err(step_error("power limits (ryzenadj)", &log)),
        3 => Err(step_error("core preset (bcdedit)", &log)),
        4 => Err(step_error("max frequency (powercfg)", &log)),
        c => Err(format!("apply failed (exit code {c})")),
    };
    let _ = std::fs::remove_file(&log);
    result
}

/// Builds an error message for a failed batch step, appending the last line of
/// the captured log when available.
fn step_error(step: &str, log: &Path) -> String {
    let detail = std::fs::read_to_string(log)
        .ok()
        .and_then(|s| s.trim().lines().last().map(str::to_string))
        .filter(|s| !s.is_empty());
    match detail {
        Some(d) => format!("{step} failed: {d}"),
        None => format!("{step} failed"),
    }
}

// ---------- Unsupported on Windows (no-op stubs) ----------

pub fn read_boost() -> Option<bool> {
    None
}
pub fn set_boost(_enabled: bool) -> Result<(), String> {
    Ok(())
}
pub fn read_smt() -> Option<bool> {
    None
}
pub fn set_smt(_enabled: bool) -> Result<(), String> {
    Ok(())
}

// ---------- Elevation ----------

fn to_wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// Runs `program` with the argument string `args` elevated via UAC ("runas"),
/// waits for it to exit, and returns Ok(()) on exit code 0.
fn run_elevated(program: &OsStr, args: &str) -> Result<(), String> {
    match run_elevated_code(program, args)? {
        0 => Ok(()),
        code => Err(format!("process exited with code {code}")),
    }
}

/// Like `run_elevated`, but returns the raw process exit code so callers can
/// distinguish between failing steps (used by the batched `apply_saved`).
fn run_elevated_code(program: &OsStr, args: &str) -> Result<u32, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let verb = to_wide(OsStr::new("runas"));
    let file = to_wide(program);
    let params = to_wide(OsStr::new(args));

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0; // SW_HIDE — no flashing console window

    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        return Err("elevation was cancelled or failed (UAC)".to_string());
    }
    if info.hProcess.is_null() {
        return Err("no process handle returned from ShellExecuteEx".to_string());
    }

    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        let mut code: u32 = 0;
        let got = GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
        if got == 0 {
            return Err("could not read process exit code".to_string());
        }
        Ok(code)
    }
}
