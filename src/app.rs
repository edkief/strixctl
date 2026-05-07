use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use egui::{Color32, RichText, Ui};

use crate::backend;
use crate::state::{AppState, PptLimits, Profile};
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
    // Visible temperature range for the fan curve graph (zoomed view)
    fan_view_temp: (f32, f32),
    // In-flight PPT reload from background thread
    ppt_reload_rx: Option<mpsc::Receiver<Option<PptLimits>>>,
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
        let fan_view_temp = fit_fan_view(&state.fan_curve.points);
        let ppt_apu_str = state.ppt.apu_limit.to_string();
        let ppt_fast_str = state.ppt.fast_limit.to_string();
        let ppt_slow_str = state.ppt.slow_limit.to_string();
        App {
            state,
            rx,
            ppt_apu_str,
            ppt_fast_str,
            ppt_slow_str,
            shift_input: String::new(),
            dragging: None,
            fan_view_temp,
            ppt_reload_rx: None,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Drain watcher channel
        while let Ok(reading) = self.rx.try_recv() {
            self.state.current_temp = reading.temp_c;
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
            ui.heading("strixctrl");
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
                    let color = if self.state.current_temp > 90.0 {
                        Color32::from_rgb(255, 80, 80)
                    } else if self.state.current_temp > 75.0 {
                        Color32::from_rgb(255, 180, 0)
                    } else {
                        Color32::from_rgb(100, 220, 100)
                    };
                    ui.colored_label(color, format!("CPU: {:.1}°C", self.state.current_temp));
                }
            });
        });
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
