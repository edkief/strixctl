use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use egui::{Color32, RichText, Ui};

use crate::backend;
use crate::profiles::{self, SavedProfile};
use crate::state::{AppState, CorePreset, PptLimits, Profile};
use crate::watcher::SensorReading;
use crate::widgets::fan_curve::FanCurveWidget;

pub struct App {
    state: AppState,
    rx: mpsc::Receiver<SensorReading>,
    // PPT input buffers (string form for text edits)
    ppt_apu_str: String,
    ppt_fast_str: String,
    ppt_slow_str: String,
    // Fan curve shift input
    shift_input: String,
    // Drag state for fan curve widget
    dragging: Option<usize>,
    // Multi-selection state for fan curve widget
    fan_selected: Vec<usize>,
    fan_select_drag: Option<f32>,
    // Visible temperature range for the fan curve graph (zoomed view)
    fan_view_temp: (f32, f32),
    // In-flight PPT reload from background thread
    ppt_reload_rx: Option<mpsc::Receiver<Option<PptLimits>>>,
    // Saved profiles
    saved_profiles: Vec<SavedProfile>,
    selected_profile_name: String,
    new_profile_name: String,
}

impl App {
    pub fn new(mut state: AppState, rx: mpsc::Receiver<SensorReading>) -> Self {
        // Sync profile and PPT limits from system on startup
        if let Some(p) = backend::read_current_profile() {
            state.profile = p;
        }
        if let Some(ppt) = backend::read_current_ppt() {
            state.ppt = ppt;
        }
        if let Some(fc) = backend::read_fan_curve(&state.profile) {
            state.fan_curve = fc;
        }
        if let Some(b) = backend::read_boost() {
            state.boost_enabled = b;
        }
        state.core_preset = backend::read_core_preset();
        let fan_view_temp = fit_fan_view(&state.fan_curve.points);
        let ppt_apu_str = state.ppt.apu_limit.to_string();
        let ppt_fast_str = state.ppt.fast_limit.to_string();
        let ppt_slow_str = state.ppt.slow_limit.to_string();
        let saved_profiles = profiles::load();
        let selected_profile_name = saved_profiles.first().map(|p| p.name.clone()).unwrap_or_default();
        App {
            state,
            rx,
            ppt_apu_str,
            ppt_fast_str,
            ppt_slow_str,
            shift_input: String::new(),
            dragging: None,
            fan_selected: Vec::new(),
            fan_select_drag: None,
            fan_view_temp,
            ppt_reload_rx: None,
            saved_profiles,
            selected_profile_name,
            new_profile_name: String::new(),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain watcher channel
        while let Ok(reading) = self.rx.try_recv() {
            self.state.current_temp = reading.temp_c;
            self.state.current_gpu_temp = reading.gpu_temp_c;
            // Auto-safety: max cooling when temp is critical
            if self.state.current_temp > 95.0 && self.state.profile != Profile::Performance {
                self.state.profile = Profile::Performance;
                match backend::apply_profile(&self.state.profile) {
                    Ok(_) => {
                        self.state.status_msg = "⚠ Critical temp! Switched to Performance.".into();
                        if let Some(fc) = backend::read_fan_curve(&self.state.profile) {
                            self.fan_view_temp = fit_fan_view(&fc.points);
                            self.state.fan_curve = fc;
                        }
                        self.reload_ppt();
                    }
                    Err(e) => self.state.status_msg = format!("⚠ Auto-profile error: {e}"),
                }
            }
        }

        // Receive completed PPT reload from background thread
        if let Some(rx) = &self.ppt_reload_rx {
            if let Ok(result) = rx.try_recv() {
                self.ppt_reload_rx = None;
                if let Some(ppt) = result {
                    self.ppt_apu_str = ppt.apu_limit.to_string();
                    self.ppt_fast_str = ppt.fast_limit.to_string();
                    self.ppt_slow_str = ppt.slow_limit.to_string();
                    self.state.ppt = ppt;
                    self.state.status_msg = "Power limits synced.".into();
                }
            }
        }

        // Request repaint every second for live temp updates
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("strixctl");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Reload").on_hover_text("Refresh all settings from system").clicked() {
                        self.reload_from_system();
                    }
                });
            });
            ui.add_space(4.0);

            self.show_profile_section(ui);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            self.show_ppt_section(ui);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            self.show_fan_curve_section(ui);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            self.show_cpu_section(ui);
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);

            self.show_profiles_section(ui);
            ui.add_space(8.0);
            ui.separator();

            self.show_status_bar(ui);
        });
    }
}

impl App {
    fn show_profile_section(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Platform Profile").strong());
        ui.horizontal(|ui| {
            for profile in [Profile::Quiet, Profile::Balanced, Profile::Performance] {
                let label = match profile {
                    Profile::Quiet => "Quiet",
                    Profile::Balanced => "Balanced",
                    Profile::Performance => "Performance",
                };
                let selected = self.state.profile == profile;
                if ui.selectable_label(selected, label).clicked() {
                    self.state.profile = profile;
                }
            }
            if ui.button("Apply").clicked() {
                match backend::apply_profile(&self.state.profile) {
                    Ok(_) => {
                        self.state.status_msg =
                            format!("Profile set to {}", self.state.profile.as_str());
                        if let Some(fc) = backend::read_fan_curve(&self.state.profile) {
                            self.fan_view_temp = fit_fan_view(&fc.points);
                            self.state.fan_curve = fc;
                        }
                        self.reload_ppt();
                    }
                    Err(e) => self.state.status_msg = format!("Error: {e}"),
                }
            }
        });
    }

    fn show_ppt_section(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Power Limits (mW)").strong());
        ui.horizontal(|ui| {
            ui.label("APU/STAPM:");
            ui.add(egui::TextEdit::singleline(&mut self.ppt_apu_str).desired_width(70.0));
            ui.label("Slow:");
            ui.add(egui::TextEdit::singleline(&mut self.ppt_slow_str).desired_width(70.0));
            ui.label("Fast:");
            ui.add(egui::TextEdit::singleline(&mut self.ppt_fast_str).desired_width(70.0));

            // Parse and validate before enabling Apply
            let parsed = (
                self.ppt_apu_str.parse::<u32>(),
                self.ppt_slow_str.parse::<u32>(),
                self.ppt_fast_str.parse::<u32>(),
            );
            let valid = match &parsed {
                (Ok(_), Ok(sl), Ok(f)) => sl <= f,
                _ => false,
            };

            let apply_btn = ui.add_enabled(valid && self.ppt_reload_rx.is_none(), egui::Button::new("Apply"));
            if apply_btn.clicked() {
                if let (Ok(a), Ok(sl), Ok(f)) = parsed {
                    self.state.ppt.apu_limit = a;
                    self.state.ppt.slow_limit = sl;
                    self.state.ppt.fast_limit = f;
                    match backend::apply_ppt(&self.state.ppt) {
                        Ok(_) => self.state.status_msg = "Power limits applied.".into(),
                        Err(e) => self.state.status_msg = format!("Error: {e}"),
                    }
                }
            }
        });

        // Inline validation warning
        let all_ok = self.ppt_apu_str.parse::<u32>().is_ok()
            && self.ppt_slow_str.parse::<u32>().is_ok()
            && self.ppt_fast_str.parse::<u32>().is_ok();
        if all_ok {
            let sl = self.ppt_slow_str.parse::<u32>().unwrap_or(0);
            let f = self.ppt_fast_str.parse::<u32>().unwrap_or(0);
            if sl > f {
                ui.colored_label(
                    Color32::from_rgb(255, 180, 0),
                    "⚠ Slow limit must not exceed Fast limit",
                );
            }
        }
    }

    fn show_fan_curve_section(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Fan Curve").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label("°C");
                ui.add(
                    egui::DragValue::new(&mut self.state.fan_curve.hysteresis)
                        .range(0u8..=10u8)
                        .speed(1),
                );
                ui.label("Hysteresis:");
                if ui.small_button("Fit").on_hover_text("Reset zoom to fit curve").clicked() {
                    self.fan_view_temp = fit_fan_view(&self.state.fan_curve.points);
                }
            });
        });

        FanCurveWidget::new(
            &mut self.state.fan_curve.points,
            self.state.current_temp,
            &mut self.dragging,
            &mut self.fan_view_temp,
            &mut self.fan_selected,
            &mut self.fan_select_drag,
        )
        .show(ui);

        ui.horizontal(|ui| {
            if ui.button("−5°C").clicked() {
                self.state.fan_curve.shift(-5);
            }
            if ui.button("+5°C").clicked() {
                self.state.fan_curve.shift(5);
            }
            ui.label("Custom offset:");
            ui.add(egui::TextEdit::singleline(&mut self.shift_input).desired_width(50.0));
            if ui.button("Shift").clicked() {
                if let Ok(delta) = self.shift_input.trim().parse::<i32>() {
                    self.state.fan_curve.shift(delta);
                    self.shift_input.clear();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Apply Curve").clicked() {
                    match backend::apply_fan_curve(&self.state.profile, &self.state.fan_curve) {
                        Ok(_) => self.state.status_msg = "Fan curve applied.".into(),
                        Err(e) => self.state.status_msg = format!("Error: {e}"),
                    }
                }
            });
        });
    }

    fn show_cpu_section(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("CPU Controls").strong());

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.state.boost_enabled, "CPU Boost");
            if ui.button("Apply").clicked() {
                match backend::set_boost(self.state.boost_enabled) {
                    Ok(_) => self.state.status_msg = format!(
                        "Boost {}.", if self.state.boost_enabled { "enabled" } else { "disabled" }
                    ),
                    Err(e) => self.state.status_msg = format!("Boost error: {e}"),
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("Active cores:");
            for preset in [CorePreset::Four, CorePreset::Eight, CorePreset::Twelve, CorePreset::Sixteen] {
                ui.radio_value(&mut self.state.core_preset, preset.clone(), preset.label());
            }
            if ui.button("Apply").clicked() {
                match backend::set_core_preset(&self.state.core_preset) {
                    Ok(_) => self.state.status_msg = format!(
                        "Core preset set to {}.", self.state.core_preset.label()
                    ),
                    Err(e) => self.state.status_msg = format!("Core preset error: {e}"),
                }
            }
        });
    }

    fn show_profiles_section(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Saved Profiles").strong());

        // Save row
        ui.horizontal(|ui| {
            ui.label("Name:");
            ui.add(egui::TextEdit::singleline(&mut self.new_profile_name).desired_width(140.0));
            let has_name = !self.new_profile_name.trim().is_empty();
            if ui.add_enabled(has_name, egui::Button::new("Save")).clicked() {
                self.save_profile();
            }
        });

        // Load / delete row
        ui.horizontal(|ui| {
            let combo_label = self
                .saved_profiles
                .iter()
                .find(|p| p.name == self.selected_profile_name)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "— select —".into());
            egui::ComboBox::from_id_salt("profile_select")
                .selected_text(combo_label)
                .width(180.0)
                .show_ui(ui, |ui| {
                    for p in &self.saved_profiles {
                        ui.selectable_value(
                            &mut self.selected_profile_name,
                            p.name.clone(),
                            p.name.clone(),
                        );
                    }
                });

            let has_selection = self
                .saved_profiles
                .iter()
                .any(|p| p.name == self.selected_profile_name);

            if ui.add_enabled(has_selection, egui::Button::new("Load")).clicked() {
                self.load_selected_profile();
            }
            if ui.add_enabled(has_selection, egui::Button::new("Apply")).clicked() {
                self.apply_selected_profile();
            }
            if ui.add_enabled(has_selection, egui::Button::new("Delete")).clicked() {
                self.saved_profiles.retain(|p| p.name != self.selected_profile_name);
                self.selected_profile_name = self
                    .saved_profiles
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                profiles::save(&self.saved_profiles);
                self.state.status_msg = "Profile deleted.".into();
            }
        });
    }

    fn save_profile(&mut self) {
        let name = self.new_profile_name.trim().to_string();
        let saved = SavedProfile {
            name: name.clone(),
            platform_profile: self.state.profile.clone(),
            ppt: self.state.ppt.clone(),
            fan_curve: self.state.fan_curve.points.clone(),
            fan_hysteresis: self.state.fan_curve.hysteresis,
            boost_enabled: self.state.boost_enabled,
            core_preset: self.state.core_preset.clone(),
        };
        profiles::upsert(&mut self.saved_profiles, saved);
        profiles::save(&self.saved_profiles);
        self.selected_profile_name = name;
        self.state.status_msg = "Profile saved.".into();
    }

    fn load_selected_profile(&mut self) {
        let Some(saved) = self
            .saved_profiles
            .iter()
            .find(|p| p.name == self.selected_profile_name)
            .cloned()
        else {
            return;
        };

        self.state.profile = saved.platform_profile;
        self.ppt_apu_str = saved.ppt.apu_limit.to_string();
        self.ppt_fast_str = saved.ppt.fast_limit.to_string();
        self.ppt_slow_str = saved.ppt.slow_limit.to_string();
        self.state.ppt = saved.ppt;
        self.state.fan_curve.points = saved.fan_curve;
        self.state.fan_curve.hysteresis = saved.fan_hysteresis;
        self.state.boost_enabled = saved.boost_enabled;
        self.state.core_preset = saved.core_preset;
        self.fan_view_temp = fit_fan_view(&self.state.fan_curve.points);
        self.state.status_msg = format!("Loaded '{}'.", self.selected_profile_name);
    }

    fn apply_selected_profile(&mut self) {
        match backend::apply_profile(&self.state.profile) {
            Ok(_) => {}
            Err(e) => { self.state.status_msg = format!("Profile error: {e}"); return; }
        }
        match backend::apply_ppt(&self.state.ppt) {
            Ok(_) => {}
            Err(e) => { self.state.status_msg = format!("PPT error: {e}"); return; }
        }
        match backend::apply_fan_curve(&self.state.profile, &self.state.fan_curve) {
            Ok(_) => {}
            Err(e) => { self.state.status_msg = format!("Fan curve error: {e}"); return; }
        }
        if let Err(e) = backend::set_boost(self.state.boost_enabled) {
            self.state.status_msg = format!("Boost error: {e}"); return;
        }
        if let Err(e) = backend::set_core_preset(&self.state.core_preset) {
            self.state.status_msg = format!("Core preset error: {e}"); return;
        }
        let name = self.selected_profile_name.clone();
        profiles::save_active(&name);
        notify_daemon_profile_applied(&name);
        self.state.status_msg = format!("Applied '{name}'.");
    }

    fn reload_from_system(&mut self) {
        if let Some(p) = backend::read_current_profile() {
            self.state.profile = p;
        }
        if let Some(ppt) = backend::read_current_ppt() {
            self.ppt_apu_str = ppt.apu_limit.to_string();
            self.ppt_fast_str = ppt.fast_limit.to_string();
            self.ppt_slow_str = ppt.slow_limit.to_string();
            self.state.ppt = ppt;
        }
        if let Some(fc) = backend::read_fan_curve(&self.state.profile) {
            self.state.fan_curve = fc;
            self.fan_view_temp = fit_fan_view(&self.state.fan_curve.points);
        }
        if let Some(b) = backend::read_boost() {
            self.state.boost_enabled = b;
        }
        self.state.core_preset = backend::read_core_preset();
        self.state.status_msg = "Settings reloaded from system.".into();
    }

    fn reload_ppt(&mut self) {
        let (tx, rx) = mpsc::channel();
        self.ppt_reload_rx = Some(rx);
        self.state.status_msg = "Syncing power limits...".into();
        // Sleep off-thread so asusd can finish its power-state transitions
        // (EPP, PPT) before ryzenadj touches the same registers.
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(800));
            let _ = tx.send(backend::read_current_ppt());
        });
    }

    fn show_status_bar(&self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(&self.state.status_msg);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.state.current_temp > 0.0 {
                    ui.colored_label(temp_color(self.state.current_temp, 90.0, 75.0),
                        format!("CPU: {:.1}°C", self.state.current_temp));
                }
                if let Some(gpu) = self.state.current_gpu_temp {
                    ui.colored_label(temp_color(gpu, 90.0, 75.0),
                        format!("GPU: {:.1}°C", gpu));
                }
            });
        });
    }
}

/// Best-effort D-Bus notification to the daemon that a saved profile was applied.
/// Spawned as a detached child — failure is silently ignored.
fn notify_daemon_profile_applied(name: &str) {
    let _ = std::process::Command::new("gdbus")
        .args([
            "call", "--session",
            "--dest", "com.strixctl.Service",
            "--object-path", "/com/strixctl/Service",
            "--method", "com.strixctl.Service.NotifyProfileApplied",
            &format!("'{}'", name.replace('\'', "\\'")),
        ])
        .spawn();
}

fn temp_color(temp: f32, critical: f32, warn: f32) -> Color32 {
    if temp > critical {
        Color32::from_rgb(255, 80, 80)
    } else if temp > warn {
        Color32::from_rgb(255, 180, 0)
    } else {
        Color32::from_rgb(100, 220, 100)
    }
}

/// Returns a temp view range that fits `points` with 15% padding on each side.
fn fit_fan_view(points: &[(f32, f32)]) -> (f32, f32) {
    if points.is_empty() {
        return (0.0, 100.0);
    }
    let min_t = points.iter().map(|(t, _)| *t).fold(f32::INFINITY, f32::min);
    let max_t = points.iter().map(|(t, _)| *t).fold(f32::NEG_INFINITY, f32::max);
    let pad = ((max_t - min_t) * 0.15).max(5.0);
    ((min_t - pad).max(0.0), (max_t + pad).min(100.0))
}
