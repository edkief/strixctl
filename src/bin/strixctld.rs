//! strixctld — session D-Bus service for strixctl.
//!
//! Bus name  : com.strixctl.Service
//! Object    : /com/strixctl/Service
//! Interface : com.strixctl.Service
//!
//! Methods
//!   ListSavedProfiles()                       -> as
//!   ApplySavedProfile(name: s)                -> (nothing, or D-Bus error)
//!   SetAsusProfile(profile: s)                -> (nothing, or D-Bus error)
//!   ApplyPpt(apu_mw: u, fast_mw: u, slow_mw: u) -> (nothing, or D-Bus error)
//!
//! Properties
//!   CurrentTempC  (read-only f64)
//!
//! Signals
//!   TempChanged(temp: f64)   — fired when temp shifts by > 0.5 °C

#[path = "../state.rs"]
mod state;
#[path = "../backend/mod.rs"]
mod backend;
#[path = "../profiles.rs"]
mod profiles;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::{connection, fdo, interface, object_server::SignalContext};

use crate::state::{CorePreset, FanCurve, PptLimits, Profile};

// ── D-Bus interface ──────────────────────────────────────────────────────────

struct StrixCtlService {
    current_temp: Arc<Mutex<f64>>,
    current_profile: Arc<Mutex<String>>,
    current_saved_profile: Arc<Mutex<String>>,
    boost_enabled: Arc<Mutex<bool>>,
    core_preset: Arc<Mutex<u32>>,
    last_applied_at: Arc<Mutex<Option<Instant>>>,
}

#[interface(name = "com.strixctl.Service")]
impl StrixCtlService {
    /// Returns the names of all saved profiles from ~/.config/strixctl/profiles.json.
    fn list_saved_profiles(&self) -> Vec<String> {
        profiles::load().into_iter().map(|p| p.name).collect()
    }

    /// Applies a saved profile by name: fan curve first, then PPT.
    ///
    /// Order matters: `asusctl fan-curve --enable-fan-curves true` causes asusd
    /// to reload its internal power state, which overwrites any PPT registers
    /// that ryzenadj already set.  We therefore apply the fan curve first, wait
    /// 800 ms for asusd to finish its transitions, then apply PPT — mirroring
    /// what the GUI does in `reload_ppt`.
    async fn apply_saved_profile(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        name: &str,
    ) -> fdo::Result<()> {
        let errors = run_apply_saved_profile(name).await;
        if errors.is_empty() {
            *self.last_applied_at.lock().unwrap() = Some(Instant::now());
            profiles::save_active(name);
            let changed = {
                let mut guard = self.current_saved_profile.lock().unwrap();
                if *guard != name {
                    *guard = name.to_string();
                    true
                } else {
                    false
                }
            };
            if changed {
                let _ = Self::saved_profile_changed(&ctxt, name).await;
            }
            Ok(())
        } else {
            Err(fdo::Error::Failed(errors.join(" | ")))
        }
    }

    /// Sets the asusctl platform profile. Accepts "quiet", "balanced", or "performance".
    fn set_asus_profile(&self, profile: &str) -> fdo::Result<()> {
        let p = match profile.to_ascii_lowercase().as_str() {
            "quiet" => Profile::Quiet,
            "balanced" => Profile::Balanced,
            "performance" => Profile::Performance,
            _ => {
                return Err(fdo::Error::InvalidArgs(format!(
                    "unknown profile '{profile}'; expected quiet|balanced|performance"
                )))
            }
        };
        backend::apply_profile(&p).map_err(fdo::Error::Failed)?;
        *self.last_applied_at.lock().unwrap() = Some(Instant::now());
        Ok(())
    }

    /// Applies PPT limits directly (values in milliwatts).
    /// slow_mw must not exceed fast_mw.
    fn apply_ppt(&self, apu_mw: u32, fast_mw: u32, slow_mw: u32) -> fdo::Result<()> {
        let ppt = PptLimits { apu_limit: apu_mw, fast_limit: fast_mw, slow_limit: slow_mw };
        if !ppt.is_valid() {
            return Err(fdo::Error::InvalidArgs(
                "slow_mw must not exceed fast_mw".into(),
            ));
        }
        backend::apply_ppt(&ppt).map_err(fdo::Error::Failed)
    }

    /// Current CPU package temperature in °C (updated every second).
    #[zbus(property)]
    fn current_temp_c(&self) -> f64 {
        *self.current_temp.lock().unwrap()
    }

    /// Current asusctl platform profile: "quiet", "balanced", or "performance".
    #[zbus(property)]
    fn current_platform_profile(&self) -> String {
        self.current_profile.lock().unwrap().clone()
    }

    /// Name of the last saved strixctl profile that was applied (empty if none).
    #[zbus(property)]
    fn current_saved_profile(&self) -> String {
        self.current_saved_profile.lock().unwrap().clone()
    }

    /// Called by the GUI (best-effort) when a saved profile is applied directly.
    async fn notify_profile_applied(&self, #[zbus(signal_context)] ctxt: SignalContext<'_>, name: &str) -> fdo::Result<()> {
        let changed = {
            let mut guard = self.current_saved_profile.lock().unwrap();
            if *guard != name {
                *guard = name.to_string();
                true
            } else {
                false
            }
        };
        if changed {
            let _ = Self::saved_profile_changed(&ctxt, name).await;
        }
        Ok(())
    }

    /// Enables or disables CPU boost.
    async fn set_boost(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        enabled: bool,
    ) -> fdo::Result<()> {
        backend::set_boost(enabled).map_err(fdo::Error::Failed)?;
        *self.boost_enabled.lock().unwrap() = enabled;
        let _ = Self::boost_changed(&ctxt, enabled).await;
        Ok(())
    }

    /// Sets the active core-count preset (4, 8, 12, or 16).
    async fn set_core_preset(
        &self,
        #[zbus(signal_context)] ctxt: SignalContext<'_>,
        preset: u32,
    ) -> fdo::Result<()> {
        let cp = CorePreset::from_u32(preset);
        backend::set_core_preset(&cp).map_err(fdo::Error::Failed)?;
        *self.core_preset.lock().unwrap() = preset;
        let _ = Self::core_preset_changed(&ctxt, preset).await;
        Ok(())
    }

    /// Whether CPU boost is currently enabled.
    #[zbus(property)]
    fn boost_enabled(&self) -> bool {
        *self.boost_enabled.lock().unwrap()
    }

    /// Active core-count preset as a number (4, 8, 12, or 16).
    #[zbus(property)]
    fn active_core_preset(&self) -> u32 {
        *self.core_preset.lock().unwrap()
    }

    /// Emitted when CPU boost is toggled.
    #[zbus(signal)]
    async fn boost_changed(ctxt: &SignalContext<'_>, enabled: bool) -> zbus::Result<()>;

    /// Emitted when the active core-count preset changes.
    #[zbus(signal)]
    async fn core_preset_changed(ctxt: &SignalContext<'_>, preset: u32) -> zbus::Result<()>;

    /// Emitted when the CPU temperature changes by more than 0.5 °C.
    #[zbus(signal)]
    async fn temp_changed(ctxt: &SignalContext<'_>, temp: f64) -> zbus::Result<()>;

    /// Emitted when the asusctl platform profile changes.
    #[zbus(signal)]
    async fn platform_profile_changed(ctxt: &SignalContext<'_>, profile: &str) -> zbus::Result<()>;

    /// Emitted when the active saved strixctl profile changes.
    #[zbus(signal)]
    async fn saved_profile_changed(ctxt: &SignalContext<'_>, name: &str) -> zbus::Result<()>;
}

// ── Shared apply logic ────────────────────────────────────────────────────────

/// Applies a saved profile by name. Returns a list of error strings (empty on
/// full success). Extracted so both the D-Bus method and the startup task can
/// share the same sequencing without duplicating code.
async fn run_apply_saved_profile(name: &str) -> Vec<String> {
    let saved = profiles::load();
    let profile = match saved.iter().find(|p| p.name == name) {
        Some(p) => p.clone(),
        None => return vec![format!("profile '{name}' not found")],
    };

    let mut errors: Vec<String> = Vec::new();

    if let Err(e) = backend::apply_profile(&profile.platform_profile) {
        errors.push(format!("platform profile: {e}"));
    }

    let curve = FanCurve { points: profile.fan_curve.clone(), hysteresis: profile.fan_hysteresis };
    if let Err(e) = backend::apply_fan_curve(&profile.platform_profile, &curve) {
        errors.push(format!("fan curve: {e}"));
    }

    // Wait for asusd to finish touching PPT registers before ryzenadj writes.
    tokio::time::sleep(Duration::from_millis(800)).await;

    if let Err(e) = backend::apply_ppt(&profile.ppt) {
        errors.push(format!("PPT: {e}"));
    }

    if let Err(e) = backend::set_boost(profile.boost_enabled) {
        errors.push(format!("boost: {e}"));
    }

    if let Err(e) = backend::set_core_preset(&profile.core_preset) {
        errors.push(format!("core preset: {e}"));
    }

    errors
}

/// PPT comparison tolerance in milliwatts. `ryzenadj --info` reports limits in
/// whole watts and firmware may round them slightly, so differences below this
/// threshold are treated as noise rather than an external change.
const PPT_TOLERANCE_MW: u32 = 500;

/// Returns true when any of the three power limits (APU/stapm, fast, slow)
/// differs from `wanted` by more than `PPT_TOLERANCE_MW`.
fn ppt_differs(current: &PptLimits, wanted: &PptLimits) -> bool {
    let off = |a: u32, b: u32| a.abs_diff(b) > PPT_TOLERANCE_MW;
    off(current.apu_limit, wanted.apu_limit)
        || off(current.fast_limit, wanted.fast_limit)
        || off(current.slow_limit, wanted.slow_limit)
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let current_temp = Arc::new(Mutex::new(0.0f64));
    let current_profile = Arc::new(Mutex::new(String::new()));
    let current_saved_profile = Arc::new(Mutex::new(
        profiles::load_active().unwrap_or_default()
    ));
    let boost_enabled = Arc::new(Mutex::new(
        backend::read_boost().unwrap_or(true)
    ));
    let core_preset = Arc::new(Mutex::new(
        backend::read_core_preset().as_u32()
    ));
    let last_applied_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    let service = StrixCtlService {
        current_temp: current_temp.clone(),
        current_profile: current_profile.clone(),
        current_saved_profile: current_saved_profile.clone(),
        boost_enabled: boost_enabled.clone(),
        core_preset: core_preset.clone(),
        last_applied_at: last_applied_at.clone(),
    };

    let conn = connection::Builder::session()?
        .name("com.strixctl.Service")?
        .serve_at("/com/strixctl/Service", service)?
        .build()
        .await?;

    // Apply the last-active saved profile on daemon startup so system settings
    // are restored after login/reboot without needing the GUI or extension.
    //
    // Delay the first apply by 30 s. At session start amdgpu is still bringing up
    // DCN/DMCUB and asusd is setting its own platform profile/EPP — all of which
    // touch the Strix Halo SMU mailbox. Firing ryzenadj (which poke the same
    // mailbox) into that window races amdgpu and hard-locks the machine, so we
    // wait for that init storm to settle before touching the SMU.
    if let Some(name) = profiles::load_active() {
        let conn_startup = conn.clone();
        let last_applied_startup = last_applied_at.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let errors = run_apply_saved_profile(&name).await;
            if errors.is_empty() {
                *last_applied_startup.lock().unwrap() = Some(Instant::now());
                eprintln!("[strixctld] startup: applied profile '{name}'");
                if let Ok(iref) = conn_startup
                    .object_server()
                    .interface::<_, StrixCtlService>("/com/strixctl/Service")
                    .await
                {
                    let _ = StrixCtlService::saved_profile_changed(
                        iref.signal_context(), &name,
                    ).await;
                }
            } else {
                eprintln!("[strixctld] startup: apply '{}' errors: {}", name, errors.join(" | "));
            }
        });
    }

    // Poll CPU temp every second; emit TempChanged when it shifts by > 0.5 °C.
    let conn_clone = conn.clone();
    tokio::spawn(async move {
        let mut last_emitted = f64::NAN;
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let Some(temp_c) = read_cpu_temp() else { continue };
            let temp = temp_c as f64;
            *current_temp.lock().unwrap() = temp;

            if last_emitted.is_nan() || (temp - last_emitted).abs() > 0.5 {
                last_emitted = temp;
                if let Ok(iref) = conn_clone
                    .object_server()
                    .interface::<_, StrixCtlService>("/com/strixctl/Service")
                    .await
                {
                    let _ = StrixCtlService::temp_changed(iref.signal_context(), temp).await;
                }
            }
        }
    });

    // Poll the asusctl platform profile every 15 s. This path is cheap and SMU-safe
    // (it queries asusd over D-Bus, never the SMU mailbox), so it can run often:
    //   1. Emits PlatformProfileChanged whenever the platform profile changes.
    //   2. Drift guard: if a saved profile is active and another process (e.g. asusd)
    //      has changed the platform profile away from what it requires, re-applies it.
    // The 15 s cooldown after any apply prevents a re-apply loop while asusd is still
    // settling its own power state after a fan-curve enable.
    let conn_clone2 = conn.clone();
    let current_saved_for_guard = current_saved_profile.clone();
    let last_applied_for_guard = last_applied_at.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            interval.tick().await;
            let Some(p) = backend::read_current_profile() else { continue };
            let name = p.as_str().to_string();
            let changed = {
                let mut guard = current_profile.lock().unwrap();
                if *guard != name {
                    *guard = name.clone();
                    true
                } else {
                    false
                }
            };
            if changed {
                if let Ok(iref) = conn_clone2
                    .object_server()
                    .interface::<_, StrixCtlService>("/com/strixctl/Service")
                    .await
                {
                    let _ = StrixCtlService::platform_profile_changed(
                        iref.signal_context(), &name,
                    ).await;
                }
            }

            let saved_name = current_saved_for_guard.lock().unwrap().clone();
            if saved_name.is_empty() {
                continue;
            }
            let cooldown_ok = last_applied_for_guard
                .lock().unwrap()
                .map_or(true, |t| t.elapsed() > Duration::from_secs(15));
            if !cooldown_ok {
                continue;
            }

            let Some(saved) = profiles::load().into_iter().find(|sp| sp.name == saved_name)
            else { continue };

            if saved.platform_profile.as_str() != name {
                eprintln!(
                    "[strixctld] platform profile externally changed; \
                     re-applying saved profile '{saved_name}'"
                );
                *last_applied_for_guard.lock().unwrap() = Some(Instant::now());
                tokio::spawn(async move {
                    let errors = run_apply_saved_profile(&saved_name).await;
                    if !errors.is_empty() {
                        eprintln!(
                            "[strixctld] guard re-apply errors: {}",
                            errors.join(" | ")
                        );
                    }
                });
            }
        }
    });

    // PPT drift guard — polled separately every 60 s because, unlike the platform
    // profile, the only way to read the live PPT limits on Strix Halo is `ryzenadj
    // --info`, which pokes the SMU mailbox shared with amdgpu. Each read risks a
    // collision, so we keep it infrequent and gate it behind an active saved profile
    // plus the 15 s cooldown so it neither spawns pkexec needlessly nor loops.
    //
    // Disabled by default: even at 60 s the SMU read can race amdgpu and hard-lock
    // the machine. Enable with `--features ppt-drift-guard` to trade that risk for
    // catching another process silently changing the APU/fast/slow power limits.
    #[cfg(feature = "ppt-drift-guard")]
    {
    let current_saved_for_ppt = current_saved_profile.clone();
    let last_applied_for_ppt = last_applied_at.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;

            let saved_name = current_saved_for_ppt.lock().unwrap().clone();
            if saved_name.is_empty() {
                continue;
            }
            let cooldown_ok = last_applied_for_ppt
                .lock().unwrap()
                .map_or(true, |t| t.elapsed() > Duration::from_secs(15));
            if !cooldown_ok {
                continue;
            }

            let Some(saved) = profiles::load().into_iter().find(|sp| sp.name == saved_name)
            else { continue };

            let ppt_drift = backend::read_current_ppt()
                .is_some_and(|cur| ppt_differs(&cur, &saved.ppt));

            if ppt_drift {
                eprintln!(
                    "[strixctld] PPT externally changed; \
                     re-applying saved profile '{saved_name}'"
                );
                *last_applied_for_ppt.lock().unwrap() = Some(Instant::now());
                tokio::spawn(async move {
                    let errors = run_apply_saved_profile(&saved_name).await;
                    if !errors.is_empty() {
                        eprintln!(
                            "[strixctld] guard re-apply errors: {}",
                            errors.join(" | ")
                        );
                    }
                });
            }
        }
    });
    }

    // Poll active-profile file every 5 s; emit SavedProfileChanged on change.
    // This is a fallback for cases where the gdbus notification from the GUI
    // was not received (e.g. daemon was not yet running).
    let conn_clone3 = conn.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            let name = profiles::load_active().unwrap_or_default();
            let changed = {
                let mut guard = current_saved_profile.lock().unwrap();
                if *guard != name {
                    *guard = name.clone();
                    true
                } else {
                    false
                }
            };
            if changed {
                if let Ok(iref) = conn_clone3
                    .object_server()
                    .interface::<_, StrixCtlService>("/com/strixctl/Service")
                    .await
                {
                    let _ = StrixCtlService::saved_profile_changed(
                        iref.signal_context(), &name,
                    ).await;
                }
            }
        }
    });

    // Block forever — the connection keeps the service alive.
    std::future::pending::<()>().await;
    unreachable!()
}

// ── Temperature reading ──────────────────────────────────────────────────────

fn read_cpu_temp() -> Option<f32> {
    for zone in 0..20u8 {
        let base = format!("/sys/class/thermal/thermal_zone{zone}");
        let zone_type = fs::read_to_string(format!("{base}/type")).unwrap_or_default();
        let t = zone_type.trim();
        if t == "x86_pkg_temp" || t.contains("cpu") {
            if let Ok(raw) = fs::read_to_string(format!("{base}/temp")) {
                if let Ok(v) = raw.trim().parse::<f32>() {
                    return Some(v / 1000.0);
                }
            }
        }
    }
    None
}
