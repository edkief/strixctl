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
//!   CurrentTempC        (read-only f64)
//!   CpuFreqMhz          (read-only u32)  — fastest core, live
//!   PowerDrawW          (read-only f64)  — battery draw, 0 on AC
//!   BatteryMinutesLeft  (read-only i32)  — -1 when unknown / on AC
//!
//! Signals
//!   TempChanged(temp: f64)   — fired when temp shifts by > 0.5 °C
//!   MetricsChanged(freq_mhz: u32, power_w: f64, battery_minutes: i32)
//!                            — fired at most 1 Hz when any of them moves

#[path = "../state.rs"]
mod state;
#[path = "../backend/mod.rs"]
mod backend;
#[path = "../profiles.rs"]
mod profiles;
#[path = "../watcher.rs"]
mod watcher;

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use zbus::{connection, fdo, interface, object_server::SignalContext};
use zbus::zvariant::OwnedFd;

use crate::state::{CorePreset, FanCurve, PptLimits, Profile};

/// Proxy for systemd-logind's sleep-state signal, used to detect resume from
/// suspend. `PrepareForSleep(true)` fires just before the machine sleeps;
/// `PrepareForSleep(false)` fires right after it wakes.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    /// A delay inhibitor is required: PrepareForSleep is only a notification,
    /// and logind does not otherwise wait for this daemon's sysfs writes.
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    #[zbus(property)]
    fn preparing_for_sleep(&self) -> zbus::Result<bool>;

    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// How long to hold off SMU-touching re-applies after resuming from suspend.
/// Mirrors the startup delay: amdgpu re-inits DCN/DMCUB on resume just like on
/// boot, and asusd re-asserts its own platform profile/EPP, so a drift guard
/// firing ryzenadj into that window risks the same mailbox collision.
const RESUME_QUIET_PERIOD: Duration = Duration::from_secs(30);

// ── D-Bus interface ──────────────────────────────────────────────────────────

struct StrixCtlService {
    current_temp: Arc<Mutex<f64>>,
    current_freq_mhz: Arc<Mutex<u32>>,
    current_power_w: Arc<Mutex<f64>>,
    battery_minutes: Arc<Mutex<i32>>,
    current_profile: Arc<Mutex<String>>,
    current_saved_profile: Arc<Mutex<String>>,
    boost_enabled: Arc<Mutex<bool>>,
    core_preset: Arc<Mutex<u32>>,
    last_applied_at: Arc<Mutex<Option<Instant>>>,
    apply_lock: Arc<AsyncMutex<()>>,
    apply_generation: Arc<AtomicU64>,
    suspend_pending: Arc<AtomicBool>,
}

impl StrixCtlService {
    fn ensure_not_suspending(&self) -> fdo::Result<()> {
        if self.suspend_pending.load(Ordering::SeqCst) {
            Err(fdo::Error::Failed(
                "system suspend is in progress; retry after resume".into(),
            ))
        } else {
            Ok(())
        }
    }
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
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        let errors = run_apply_saved_profile(name).await;
        if errors.is_empty() {
            self.apply_generation.fetch_add(1, Ordering::SeqCst);
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
    async fn set_asus_profile(&self, profile: &str) -> fdo::Result<()> {
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
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        backend::apply_profile(&p).map_err(fdo::Error::Failed)?;
        self.apply_generation.fetch_add(1, Ordering::SeqCst);
        *self.last_applied_at.lock().unwrap() = Some(Instant::now());
        Ok(())
    }

    /// Applies PPT limits directly (values in milliwatts).
    /// slow_mw must not exceed fast_mw.
    async fn apply_ppt(&self, apu_mw: u32, fast_mw: u32, slow_mw: u32) -> fdo::Result<()> {
        let ppt = PptLimits { apu_limit: apu_mw, fast_limit: fast_mw, slow_limit: slow_mw };
        if !ppt.is_valid() {
            return Err(fdo::Error::InvalidArgs(
                "slow_mw must not exceed fast_mw".into(),
            ));
        }
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        backend::apply_ppt(&ppt).map_err(fdo::Error::Failed)?;
        self.apply_generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
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
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        self.apply_generation.fetch_add(1, Ordering::SeqCst);
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
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        backend::set_boost(enabled).map_err(fdo::Error::Failed)?;
        self.apply_generation.fetch_add(1, Ordering::SeqCst);
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
        let _guard = self.apply_lock.lock().await;
        self.ensure_not_suspending()?;
        backend::set_core_preset(&cp).map_err(fdo::Error::Failed)?;
        self.apply_generation.fetch_add(1, Ordering::SeqCst);
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

    /// Fastest live core frequency in MHz; 0 when unknown.
    #[zbus(property)]
    fn cpu_freq_mhz(&self) -> u32 {
        *self.current_freq_mhz.lock().unwrap()
    }

    /// Battery discharge in watts; 0.0 while on AC.
    #[zbus(property)]
    fn power_draw_w(&self) -> f64 {
        *self.current_power_w.lock().unwrap()
    }

    /// Minutes of battery left at the current draw; -1 on AC or when unknown.
    #[zbus(property)]
    fn battery_minutes_left(&self) -> i32 {
        *self.battery_minutes.lock().unwrap()
    }

    /// Emitted when the CPU temperature changes by more than 0.5 °C.
    #[zbus(signal)]
    async fn temp_changed(ctxt: &SignalContext<'_>, temp: f64) -> zbus::Result<()>;

    /// Emitted when frequency, power draw, or battery estimate moves. Carries
    /// all three so a panel display can refresh from a single signal.
    #[zbus(signal)]
    async fn metrics_changed(
        ctxt: &SignalContext<'_>,
        freq_mhz: u32,
        power_w: f64,
        battery_minutes: i32,
    ) -> zbus::Result<()>;

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

    // Pre-suspend preparation may temporarily enable SMT so every present CPU
    // can be online. Restore the saved SMT choice before its core preset.
    if let Err(e) = backend::set_smt(profile.smt_enabled) {
        errors.push(format!("SMT: {e}"));
    }

    if let Err(e) = backend::set_core_preset(&profile.core_preset) {
        errors.push(format!("core preset: {e}"));
    }

    // Absent in profiles saved before the cap existed — leave the cap alone then.
    if let Some(khz) = profile.max_freq_khz {
        if let Err(e) = backend::set_max_freq_khz(Some(khz)) {
            errors.push(format!("max frequency: {e}"));
        }
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
    let current_freq_mhz = Arc::new(Mutex::new(0u32));
    let current_power_w = Arc::new(Mutex::new(0.0f64));
    let battery_minutes = Arc::new(Mutex::new(-1i32));
    let last_applied_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let resume_quiet_until: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let apply_lock = Arc::new(AsyncMutex::new(()));
    let apply_generation = Arc::new(AtomicU64::new(0));
    let suspend_pending = Arc::new(AtomicBool::new(false));

    let service = StrixCtlService {
        current_temp: current_temp.clone(),
        current_freq_mhz: current_freq_mhz.clone(),
        current_power_w: current_power_w.clone(),
        battery_minutes: battery_minutes.clone(),
        current_profile: current_profile.clone(),
        current_saved_profile: current_saved_profile.clone(),
        boost_enabled: boost_enabled.clone(),
        core_preset: core_preset.clone(),
        last_applied_at: last_applied_at.clone(),
        apply_lock: apply_lock.clone(),
        apply_generation: apply_generation.clone(),
        suspend_pending: suspend_pending.clone(),
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
        let apply_lock_startup = apply_lock.clone();
        let apply_generation_startup = apply_generation.clone();
        let resume_quiet_startup = resume_quiet_until.clone();
        let suspend_pending_startup = suspend_pending.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let _guard = apply_lock_startup.lock().await;
            let resume_quiet = resume_quiet_startup
                .lock().unwrap()
                .is_some_and(|deadline| Instant::now() < deadline);
            if suspend_pending_startup.load(Ordering::SeqCst) || resume_quiet {
                eprintln!("[strixctld] startup apply deferred to resume restoration");
                return;
            }
            let errors = run_apply_saved_profile(&name).await;
            if errors.is_empty() {
                apply_generation_startup.fetch_add(1, Ordering::SeqCst);
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

    // Hold a logind delay inhibitor while preparing for suspend. A
    // PrepareForSleep(true) signal alone is not a synchronization barrier:
    // without the inhibitor, logind may enter sleep while cpuctl is still
    // bringing CPUs online.
    {
        let resume_quiet_until = resume_quiet_until.clone();
        let apply_lock_for_sleep = apply_lock.clone();
        let apply_generation_for_sleep = apply_generation.clone();
        let core_preset_for_sleep = core_preset.clone();
        let last_applied_for_sleep = last_applied_at.clone();
        let suspend_pending_for_sleep = suspend_pending.clone();
        tokio::spawn(async move {
            let system_conn = match connection::Builder::system() {
                Ok(builder) => match builder.build().await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("[strixctld] system bus connect failed, resume guard disabled: {e}");
                        return;
                    }
                },
                Err(e) => {
                    eprintln!("[strixctld] system bus builder failed, resume guard disabled: {e}");
                    return;
                }
            };
            let Ok(login1) = Login1ManagerProxy::new(&system_conn).await else {
                eprintln!("[strixctld] logind proxy unavailable; suspend CPU safety disabled");
                return;
            };
            let mut inhibitor = match login1.inhibit(
                "sleep",
                "strixctld",
                "bring all CPUs online before s2idle",
                "delay",
            ).await {
                Ok(fd) => Some(fd),
                Err(e) => {
                    eprintln!("[strixctld] CRITICAL: cannot acquire logind sleep delay inhibitor; suspend CPU safety disabled: {e}");
                    return;
                }
            };
            let Ok(mut sleep_signals) = login1.receive_prepare_for_sleep().await else {
                eprintln!("[strixctld] logind PrepareForSleep subscribe failed; suspend CPU safety disabled");
                return;
            };
            // Close the startup/reconnect race between acquiring the inhibitor
            // and subscribing: if logind is already preparing, synthesize the
            // true edge instead of waiting for a signal that already fired.
            let mut pending_start = match login1.preparing_for_sleep().await {
                Ok(preparing) => preparing,
                Err(e) => {
                    eprintln!("[strixctld] cannot query logind PreparingForSleep; suspend CPU safety disabled: {e}");
                    return;
                }
            };
            // The guard is intentionally retained from PrepareForSleep(true)
            // until the matching false signal. That serializes daemon-owned
            // profile/core changes across the complete sleep cycle.
            let mut sleep_guard: Option<OwnedMutexGuard<()>> = None;
            let mut restore_snapshot: Option<(u32, Option<bool>, u64)> = None;
            loop {
                let start = if pending_start {
                    pending_start = false;
                    true
                } else {
                    let Some(signal) = sleep_signals.next().await else { break };
                    let Ok(args) = signal.args() else {
                        eprintln!("[strixctld] malformed PrepareForSleep signal ignored");
                        continue;
                    };
                    args.start
                };
                if start {
                    if suspend_pending_for_sleep.swap(true, Ordering::SeqCst) {
                        eprintln!("[strixctld] duplicate PrepareForSleep(true) ignored");
                        continue;
                    }
                    if inhibitor.is_none() {
                        eprintln!("[strixctld] CRITICAL: PrepareForSleep(true) received without a delay inhibitor");
                    }

                    let guard = apply_lock_for_sleep.clone().lock_owned().await;
                    let preset = *core_preset_for_sleep.lock().unwrap();
                    let smt_enabled = backend::read_smt();
                    let generation = apply_generation_for_sleep.load(Ordering::SeqCst);
                    restore_snapshot = Some((preset, smt_enabled, generation));

                    eprintln!("[strixctld] suspend requested; bringing every present CPU online");
                    let online_result = tokio::task::spawn_blocking(backend::online_all_cpus).await;
                    match online_result {
                        Ok(Ok(())) => eprintln!("[strixctld] pre-suspend CPU online verification succeeded"),
                        Ok(Err(e)) => eprintln!("[strixctld] CRITICAL: pre-suspend CPU online verification failed; suspend will proceed after releasing the delay inhibitor: {e}"),
                        Err(e) => eprintln!("[strixctld] CRITICAL: pre-suspend CPU online task failed; suspend will proceed after releasing the delay inhibitor: {e}"),
                    }
                    sleep_guard = Some(guard);
                    // Closing the fd tells logind this delay inhibitor has
                    // completed. It is reacquired after resume for the next cycle.
                    drop(inhibitor.take());
                } else if let Some(guard) = sleep_guard.take() {
                    eprintln!("[strixctld] resume from sleep detected; holding SMU guards for {}s", RESUME_QUIET_PERIOD.as_secs());
                    *resume_quiet_until.lock().unwrap() = Some(Instant::now() + RESUME_QUIET_PERIOD);
                    suspend_pending_for_sleep.store(false, Ordering::SeqCst);
                    drop(guard);

                    if let Some((preset, smt_enabled, generation)) = restore_snapshot.take() {
                        let lock = apply_lock_for_sleep.clone();
                        let generation_counter = apply_generation_for_sleep.clone();
                        let last_applied = last_applied_for_sleep.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep(RESUME_QUIET_PERIOD).await;
                            let _guard = lock.lock().await;
                            if generation_counter.load(Ordering::SeqCst) != generation {
                                eprintln!("[strixctld] resume restore skipped; a newer profile/core change superseded the pre-suspend selection");
                                return;
                            }
                            let errors = if let Some(active_name) = profiles::load_active() {
                                run_apply_saved_profile(&active_name).await
                            } else {
                                let mut errors = Vec::new();
                                if let Some(enabled) = smt_enabled {
                                    if let Err(e) = backend::set_smt(enabled) {
                                        errors.push(format!("SMT: {e}"));
                                    }
                                }
                                if let Err(e) = backend::set_core_preset(&CorePreset::from_u32(preset)) {
                                    errors.push(format!("core preset: {e}"));
                                }
                                errors
                            };
                            if errors.is_empty() {
                                generation_counter.fetch_add(1, Ordering::SeqCst);
                                *last_applied.lock().unwrap() = Some(Instant::now());
                                eprintln!("[strixctld] resume: restored pre-suspend profile/core selection");
                            } else {
                                eprintln!("[strixctld] resume restore errors: {}", errors.join(" | "));
                            }
                        });
                    }

                    inhibitor = match login1.inhibit(
                        "sleep", "strixctld", "bring all CPUs online before s2idle", "delay",
                    ).await {
                        Ok(fd) => Some(fd),
                        Err(e) => {
                            eprintln!("[strixctld] CRITICAL: cannot reacquire logind sleep delay inhibitor; future suspend CPU safety disabled: {e}");
                            None
                        }
                    };
                } else {
                    eprintln!("[strixctld] duplicate or unmatched PrepareForSleep(false) ignored");
                }
            }
            suspend_pending_for_sleep.store(false, Ordering::SeqCst);
            eprintln!("[strixctld] logind PrepareForSleep stream ended; suspend CPU safety disabled");
        });
    }

    // Poll sensors every second; emit TempChanged when the temperature shifts by
    // > 0.5 °C and MetricsChanged when frequency, power draw, or the battery
    // estimate moves. All of it is plain sysfs reads — never the SMU mailbox —
    // so this loop is safe to run at 1 Hz regardless of the drift guards.
    let conn_clone = conn.clone();
    tokio::spawn(async move {
        let mut last_temp = f64::NAN;
        let mut last_metrics: Option<(u32, f64, i32)> = None;
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let reading = watcher::read_now();

            let temp = reading.temp_c as f64;
            if temp > 0.0 {
                *current_temp.lock().unwrap() = temp;
            }
            let freq = reading.cpu_freq_mhz.unwrap_or(0);
            let power = reading.battery_discharge_w.unwrap_or(0.0) as f64;
            let minutes = reading.battery_minutes_left.map_or(-1, |m| m as i32);
            *current_freq_mhz.lock().unwrap() = freq;
            *current_power_w.lock().unwrap() = power;
            *battery_minutes.lock().unwrap() = minutes;

            let temp_moved = temp > 0.0 && (last_temp.is_nan() || (temp - last_temp).abs() > 0.5);
            // Thresholds keep a busy CPU from emitting a signal for every MHz of
            // jitter while still tracking real changes at 1 Hz.
            let metrics_moved = match last_metrics {
                None => true,
                Some((f, p, m)) => {
                    freq.abs_diff(f) >= 50 || (power - p).abs() >= 0.3 || minutes != m
                }
            };
            if !temp_moved && !metrics_moved {
                continue;
            }

            let Ok(iref) = conn_clone
                .object_server()
                .interface::<_, StrixCtlService>("/com/strixctl/Service")
                .await
            else { continue };

            if temp_moved {
                last_temp = temp;
                let _ = StrixCtlService::temp_changed(iref.signal_context(), temp).await;
            }
            if metrics_moved {
                last_metrics = Some((freq, power, minutes));
                let _ = StrixCtlService::metrics_changed(
                    iref.signal_context(), freq, power, minutes,
                ).await;
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
    let resume_quiet_for_guard = resume_quiet_until.clone();
    let apply_lock_for_guard = apply_lock.clone();
    let apply_generation_for_guard = apply_generation.clone();
    let suspend_pending_for_guard = suspend_pending.clone();
    tokio::spawn(async move {
        // Offset the first tick by a full period. The read itself is SMU-safe, but a
        // drift re-apply runs asusctl/ryzenadj; deferring the first tick keeps any
        // re-apply out of the startup window the 30 s apply delay is protecting.
        let mut interval =
            tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(15),
                                     Duration::from_secs(15));
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
            let resume_quiet_ok = resume_quiet_for_guard
                .lock().unwrap()
                .map_or(true, |deadline| Instant::now() >= deadline);
            if !cooldown_ok || !resume_quiet_ok {
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
                let lock = apply_lock_for_guard.clone();
                let generation = apply_generation_for_guard.clone();
                let quiet = resume_quiet_for_guard.clone();
                let suspending = suspend_pending_for_guard.clone();
                tokio::spawn(async move {
                    let _guard = lock.lock().await;
                    let resume_quiet = quiet.lock().unwrap()
                        .is_some_and(|deadline| Instant::now() < deadline);
                    if suspending.load(Ordering::SeqCst) || resume_quiet {
                        return;
                    }
                    let errors = run_apply_saved_profile(&saved_name).await;
                    if errors.is_empty() {
                        generation.fetch_add(1, Ordering::SeqCst);
                    } else {
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
    let resume_quiet_for_ppt = resume_quiet_until.clone();
    let apply_lock_for_ppt = apply_lock.clone();
    let apply_generation_for_ppt = apply_generation.clone();
    let suspend_pending_for_ppt = suspend_pending.clone();
    tokio::spawn(async move {
        // Offset the first tick by a full period. `tokio::time::interval` fires its
        // first tick immediately, which would poke the SMU via `ryzenadj --info` at
        // t≈0 — right in amdgpu's init window — defeating the startup delay. Start
        // the first read 60 s out instead.
        let mut interval =
            tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(60),
                                     Duration::from_secs(60));
        loop {
            interval.tick().await;

            let saved_name = current_saved_for_ppt.lock().unwrap().clone();
            if saved_name.is_empty() {
                continue;
            }
            let cooldown_ok = last_applied_for_ppt
                .lock().unwrap()
                .map_or(true, |t| t.elapsed() > Duration::from_secs(15));
            let resume_quiet_ok = resume_quiet_for_ppt
                .lock().unwrap()
                .map_or(true, |deadline| Instant::now() >= deadline);
            if !cooldown_ok || !resume_quiet_ok {
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
                let lock = apply_lock_for_ppt.clone();
                let generation = apply_generation_for_ppt.clone();
                let quiet = resume_quiet_for_ppt.clone();
                let suspending = suspend_pending_for_ppt.clone();
                tokio::spawn(async move {
                    let _guard = lock.lock().await;
                    let resume_quiet = quiet.lock().unwrap()
                        .is_some_and(|deadline| Instant::now() < deadline);
                    if suspending.load(Ordering::SeqCst) || resume_quiet {
                        return;
                    }
                    let errors = run_apply_saved_profile(&saved_name).await;
                    if errors.is_empty() {
                        generation.fetch_add(1, Ordering::SeqCst);
                    } else {
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
    let apply_generation_for_active = apply_generation.clone();
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
                apply_generation_for_active.fetch_add(1, Ordering::SeqCst);
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
