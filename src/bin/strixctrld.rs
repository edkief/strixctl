//! strixctrld — session D-Bus service for strixctrl.
//!
//! Bus name  : com.strixctrl.Service
//! Object    : /com/strixctrl/Service
//! Interface : com.strixctrl.Service
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

struct StrixCtrlService {
    current_temp: Arc<Mutex<f64>>,
}

#[interface(name = "com.strixctrl.Service")]
impl StrixCtrlService {
    /// Returns the names of all saved profiles from ~/.config/strixctrl/profiles.json.
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

        // 1. Fan curve — triggers asusd power-state reload.
        if let Some(points) = &profile.fan_curve {
            let asus_profile = backend::read_current_profile().unwrap_or_default();
            let curve = FanCurve { points: points.clone(), hysteresis: 2 };
            if let Err(e) = backend::apply_fan_curve(&asus_profile, &curve) {
                errors.push(format!("fan curve: {e}"));
            }
        }

        // 2. Wait for asusd to finish touching PPT registers before ryzenadj
        //    writes its own values.  Only needed when both parts are present.
        if profile.fan_curve.is_some() && profile.ppt.is_some() {
            tokio::time::sleep(Duration::from_millis(800)).await;
        }

        // 3. PPT last, so asusd cannot clobber it.
        if let Some(ppt) = &profile.ppt {
            if let Err(e) = backend::apply_ppt(ppt) {
                errors.push(format!("PPT: {e}"));
            }
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

    /// Emitted when the CPU temperature changes by more than 0.5 °C.
    #[zbus(signal)]
    async fn temp_changed(ctxt: &SignalContext<'_>, temp: f64) -> zbus::Result<()>;
}

// ── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> zbus::Result<()> {
    let current_temp = Arc::new(Mutex::new(0.0f64));

    let service = StrixCtrlService { current_temp: current_temp.clone() };

    let conn = connection::Builder::session()?
        .name("com.strixctrl.Service")?
        .serve_at("/com/strixctrl/Service", service)?
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
                    .interface::<_, StrixCtrlService>("/com/strixctrl/Service")
                    .await
                {
                    let _ = StrixCtrlService::temp_changed(iref.signal_context(), temp).await;
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
