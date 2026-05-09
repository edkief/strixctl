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
#[path = "../backend.rs"]
mod backend;
#[path = "../profiles.rs"]
mod profiles;

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::{connection, fdo, interface, object_server::SignalContext};

use crate::state::{FanCurve, PptLimits, Profile};

// ── D-Bus interface ──────────────────────────────────────────────────────────

struct StrixCtlService {
    current_temp: Arc<Mutex<f64>>,
    current_profile: Arc<Mutex<String>>,
    current_saved_profile: Arc<Mutex<String>>,
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
    async fn apply_saved_profile(&self, name: &str) -> fdo::Result<()> {
        let saved = profiles::load();
        let profile = saved
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| fdo::Error::Failed(format!("profile '{name}' not found")))?
            .clone();

        let mut errors: Vec<String> = Vec::new();

        // 1. Platform profile.
        if let Err(e) = backend::apply_profile(&profile.platform_profile) {
            errors.push(format!("platform profile: {e}"));
        }

        // 2. Fan curve — triggers asusd power-state reload.
        let curve = FanCurve { points: profile.fan_curve.clone(), hysteresis: profile.fan_hysteresis };
        if let Err(e) = backend::apply_fan_curve(&profile.platform_profile, &curve) {
            errors.push(format!("fan curve: {e}"));
        }

        // 3. Wait for asusd to finish touching PPT registers before ryzenadj
        //    writes its own values.
        tokio::time::sleep(Duration::from_millis(800)).await;

        // 4. PPT last, so asusd cannot clobber it.
        if let Err(e) = backend::apply_ppt(&profile.ppt) {
            errors.push(format!("PPT: {e}"));
        }

        if errors.is_empty() {
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
        backend::apply_profile(&p).map_err(fdo::Error::Failed)
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

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let current_temp = Arc::new(Mutex::new(0.0f64));
    let current_profile = Arc::new(Mutex::new(String::new()));
    let current_saved_profile = Arc::new(Mutex::new(
        profiles::load_active().unwrap_or_default()
    ));

    let service = StrixCtlService {
        current_temp: current_temp.clone(),
        current_profile: current_profile.clone(),
        current_saved_profile: current_saved_profile.clone(),
    };

    let conn = connection::Builder::session()?
        .name("com.strixctl.Service")?
        .serve_at("/com/strixctl/Service", service)?
        .build()
        .await?;

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

    // Poll asusctl platform profile every 5 s; emit PlatformProfileChanged on change.
    let conn_clone2 = conn.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
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
        }
    });

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
