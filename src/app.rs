use std::time::Duration;

use iced::widget::{button, column, container, row, text, Space};
use iced::{Alignment, Element, Length, Subscription, Task, Theme};

use crate::backend;
use crate::profiles::{self, SavedProfile};
use crate::state::{AppState, CorePreset, PptLimits, Profile};
use crate::theme;
use crate::views;
use crate::watcher;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Profile,
    Cooling,
    Saved,
}

impl Tab {
    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Profile => "Power",
            Tab::Cooling => "Cooling",
            Tab::Saved => "Profiles",
        }
    }

    pub const ALL: [Tab; 4] = [Tab::Overview, Tab::Profile, Tab::Cooling, Tab::Saved];
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    TabSelected(Tab),
    Reload,

    // Profile
    SetProfile(Profile),
    ApplyProfile,
    ProfileApplied(Result<Profile, String>),
    PptReloaded(Option<PptLimits>),
    FanCurveReloaded(Option<crate::state::FanCurve>),

    // PPT
    PptApuChanged(String),
    PptSlowChanged(String),
    PptFastChanged(String),
    ApplyPpt,
    PptApplied(Result<(), String>),

    // Fan curve
    HysteresisChanged(String),
    FanShift(i32),
    ShiftInputChanged(String),
    ApplyShift,
    ApplyFanCurve,
    FanCurveApplied(Result<(), String>),
    FanPointDragged(usize, f32, f32),
    FanFit,

    // CPU
    BoostToggled(bool),
    ApplyBoost,
    BoostApplied(Result<bool, String>),
    SmtToggled(bool),
    ApplySmt,
    SmtApplied(Result<bool, String>),
    SetCorePreset(CorePreset),
    ApplyCorePreset,
    CorePresetApplied(Result<CorePreset, String>),

    // Saved profiles
    NewProfileNameChanged(String),
    SaveProfile,
    SelectProfile(String),
    LoadProfile,
    ApplySavedProfile,
    SavedProfileApplied(Result<String, String>),
    ReplaceProfile,
    DeleteProfile,
}

pub struct App {
    pub state: AppState,
    pub tab: Tab,
    pub ppt_apu_str: String,
    pub ppt_slow_str: String,
    pub ppt_fast_str: String,
    pub hyst_str: String,
    pub shift_input: String,
    pub fan_view_temp: (f32, f32),
    pub ppt_reload_inflight: bool,
    pub saved_profiles: Vec<SavedProfile>,
    pub selected_profile_name: String,
    pub new_profile_name: String,
    pub status_token: u64,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let mut state = AppState::default();
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
        if let Some(s) = backend::read_smt() {
            state.smt_enabled = s;
        }
        state.core_preset = backend::read_core_preset();

        let fan_view_temp = fit_fan_view(&state.fan_curve.points);
        let ppt_apu_str = state.ppt.apu_limit.to_string();
        let ppt_slow_str = state.ppt.slow_limit.to_string();
        let ppt_fast_str = state.ppt.fast_limit.to_string();
        let hyst_str = state.fan_curve.hysteresis.to_string();
        let saved_profiles = profiles::load();
        let selected_profile_name = saved_profiles
            .first()
            .map(|p| p.name.clone())
            .unwrap_or_default();

        (
            App {
                state,
                tab: Tab::Overview,
                ppt_apu_str,
                ppt_slow_str,
                ppt_fast_str,
                hyst_str,
                shift_input: String::new(),
                fan_view_temp,
                ppt_reload_inflight: false,
                saved_profiles,
                selected_profile_name,
                new_profile_name: String::new(),
                status_token: 0,
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        theme::mocha()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                let r = watcher::read_now();
                self.state.current_temp = r.temp_c;
                self.state.current_gpu_temp = r.gpu_temp_c;
                // Auto-safety
                if r.temp_c > 95.0 && self.state.profile != Profile::Performance {
                    self.state.profile = Profile::Performance;
                    self.set_status("⚠ Critical temp! Switched to Performance.");
                    let profile = self.state.profile.clone();
                    return spawn_blocking(
                        move || backend::apply_profile(&profile).map(|_| profile),
                        Message::ProfileApplied,
                    );
                }
                Task::none()
            }

            Message::TabSelected(t) => {
                self.tab = t;
                Task::none()
            }

            Message::Reload => {
                if let Some(p) = backend::read_current_profile() {
                    self.state.profile = p;
                }
                if let Some(ppt) = backend::read_current_ppt() {
                    self.set_ppt(ppt);
                }
                if let Some(fc) = backend::read_fan_curve(&self.state.profile) {
                    self.fan_view_temp = fit_fan_view(&fc.points);
                    self.hyst_str = fc.hysteresis.to_string();
                    self.state.fan_curve = fc;
                }
                if let Some(b) = backend::read_boost() {
                    self.state.boost_enabled = b;
                }
                if let Some(s) = backend::read_smt() {
                    self.state.smt_enabled = s;
                }
                self.state.core_preset = backend::read_core_preset();
                self.set_status("Settings reloaded from system.");
                Task::none()
            }

            Message::SetProfile(p) => {
                self.state.profile = p;
                Task::none()
            }

            Message::ApplyProfile => {
                let profile = self.state.profile.clone();
                self.set_status("Applying profile…");
                spawn_blocking(
                    move || backend::apply_profile(&profile).map(|_| profile),
                    Message::ProfileApplied,
                )
            }

            Message::ProfileApplied(Ok(p)) => {
                self.state.profile = p.clone();
                self.set_status(&format!("Profile set to {}.", p.as_str()));
                let profile = p.clone();
                let fan_task = spawn_blocking(
                    move || backend::read_fan_curve(&profile),
                    Message::FanCurveReloaded,
                );
                let ppt_task = self.start_ppt_reload();
                Task::batch([fan_task, ppt_task])
            }
            Message::ProfileApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }

            Message::FanCurveReloaded(Some(fc)) => {
                self.fan_view_temp = fit_fan_view(&fc.points);
                self.hyst_str = fc.hysteresis.to_string();
                self.state.fan_curve = fc;
                Task::none()
            }
            Message::FanCurveReloaded(None) => Task::none(),

            Message::PptReloaded(opt) => {
                self.ppt_reload_inflight = false;
                if let Some(ppt) = opt {
                    self.set_ppt(ppt);
                    self.set_status("Power limits synced.");
                }
                Task::none()
            }

            Message::PptApuChanged(s) => {
                self.ppt_apu_str = s;
                Task::none()
            }
            Message::PptSlowChanged(s) => {
                self.ppt_slow_str = s;
                Task::none()
            }
            Message::PptFastChanged(s) => {
                self.ppt_fast_str = s;
                Task::none()
            }

            Message::ApplyPpt => {
                if let (Ok(a), Ok(sl), Ok(f)) = (
                    self.ppt_apu_str.parse::<u32>(),
                    self.ppt_slow_str.parse::<u32>(),
                    self.ppt_fast_str.parse::<u32>(),
                ) {
                    if sl <= f {
                        self.state.ppt = PptLimits {
                            apu_limit: a,
                            slow_limit: sl,
                            fast_limit: f,
                        };
                        let ppt = self.state.ppt.clone();
                        self.set_status("Applying power limits…");
                        return spawn_blocking(
                            move || backend::apply_ppt(&ppt),
                            Message::PptApplied,
                        );
                    }
                }
                Task::none()
            }
            Message::PptApplied(Ok(())) => {
                self.set_status("Power limits applied.");
                Task::none()
            }
            Message::PptApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }

            Message::HysteresisChanged(s) => {
                if let Ok(v) = s.parse::<u8>() {
                    self.state.fan_curve.hysteresis = v.min(10);
                }
                self.hyst_str = s;
                Task::none()
            }
            Message::FanShift(delta) => {
                self.state.fan_curve.shift(delta);
                Task::none()
            }
            Message::ShiftInputChanged(s) => {
                self.shift_input = s;
                Task::none()
            }
            Message::ApplyShift => {
                if let Ok(d) = self.shift_input.trim().parse::<i32>() {
                    self.state.fan_curve.shift(d);
                }
                self.shift_input.clear();
                Task::none()
            }
            Message::ApplyFanCurve => {
                let profile = self.state.profile.clone();
                let curve = self.state.fan_curve.clone();
                self.set_status("Applying fan curve…");
                spawn_blocking(
                    move || backend::apply_fan_curve(&profile, &curve),
                    Message::FanCurveApplied,
                )
            }
            Message::FanCurveApplied(Ok(())) => {
                self.set_status("Fan curve applied.");
                Task::none()
            }
            Message::FanCurveApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }
            Message::FanPointDragged(idx, t, s) => {
                let len = self.state.fan_curve.points.len();
                if idx < len {
                    let min_t = if idx == 0 {
                        0.0
                    } else {
                        self.state.fan_curve.points[idx - 1].0 + 0.5
                    };
                    let max_t = if idx + 1 == len {
                        100.0
                    } else {
                        self.state.fan_curve.points[idx + 1].0 - 0.5
                    };
                    // Fan % must rise monotonically — clamp against neighbours
                    // so a point can never dip below the previous one or rise
                    // above the next.
                    let min_s = if idx == 0 {
                        0.0
                    } else {
                        self.state.fan_curve.points[idx - 1].1
                    };
                    let max_s = if idx + 1 == len {
                        100.0
                    } else {
                        self.state.fan_curve.points[idx + 1].1
                    };
                    let p = &mut self.state.fan_curve.points[idx];
                    p.0 = t.clamp(min_t, max_t);
                    p.1 = s.clamp(min_s, max_s);
                }
                Task::none()
            }
            Message::FanFit => {
                self.fan_view_temp = fit_fan_view(&self.state.fan_curve.points);
                Task::none()
            }

            Message::BoostToggled(b) => {
                self.state.boost_enabled = b;
                Task::none()
            }
            Message::ApplyBoost => {
                let b = self.state.boost_enabled;
                self.set_status("Applying boost…");
                spawn_blocking(move || backend::set_boost(b).map(|_| b), Message::BoostApplied)
            }
            Message::BoostApplied(Ok(b)) => {
                self.set_status(if b { "Boost enabled." } else { "Boost disabled." });
                Task::none()
            }
            Message::BoostApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }

            Message::SmtToggled(b) => {
                self.state.smt_enabled = b;
                Task::none()
            }
            Message::ApplySmt => {
                let b = self.state.smt_enabled;
                self.set_status("Applying SMT…");
                spawn_blocking(move || backend::set_smt(b).map(|_| b), Message::SmtApplied)
            }
            Message::SmtApplied(Ok(b)) => {
                self.set_status(if b { "SMT enabled." } else { "SMT disabled." });
                // After SMT changes, re-detect the active core preset since sibling
                // threads come and go from sysfs.
                self.state.core_preset = backend::read_core_preset();
                Task::none()
            }
            Message::SmtApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }

            Message::SetCorePreset(p) => {
                self.state.core_preset = p;
                Task::none()
            }
            Message::ApplyCorePreset => {
                let p = self.state.core_preset.clone();
                self.set_status("Applying core preset…");
                spawn_blocking(
                    move || backend::set_core_preset(&p).map(|_| p),
                    Message::CorePresetApplied,
                )
            }
            Message::CorePresetApplied(Ok(p)) => {
                self.set_status(&format!("Core preset set to {}.", p.label()));
                Task::none()
            }
            Message::CorePresetApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }

            Message::NewProfileNameChanged(s) => {
                self.new_profile_name = s;
                Task::none()
            }
            Message::SaveProfile => {
                let name = self.new_profile_name.trim().to_string();
                if name.is_empty() {
                    return Task::none();
                }
                let saved = SavedProfile {
                    name: name.clone(),
                    platform_profile: self.state.profile.clone(),
                    ppt: self.state.ppt.clone(),
                    fan_curve: self.state.fan_curve.points.clone(),
                    fan_hysteresis: self.state.fan_curve.hysteresis,
                    boost_enabled: self.state.boost_enabled,
                    smt_enabled: self.state.smt_enabled,
                    core_preset: self.state.core_preset.clone(),
                };
                profiles::upsert(&mut self.saved_profiles, saved);
                profiles::save(&self.saved_profiles);
                self.selected_profile_name = name;
                self.new_profile_name.clear();
                self.set_status("Profile saved.");
                Task::none()
            }
            Message::SelectProfile(n) => {
                self.selected_profile_name = n;
                Task::none()
            }
            Message::LoadProfile => {
                if let Some(saved) = self
                    .saved_profiles
                    .iter()
                    .find(|p| p.name == self.selected_profile_name)
                    .cloned()
                {
                    self.state.profile = saved.platform_profile;
                    self.set_ppt(saved.ppt);
                    self.state.fan_curve.points = saved.fan_curve;
                    self.state.fan_curve.hysteresis = saved.fan_hysteresis;
                    self.hyst_str = saved.fan_hysteresis.to_string();
                    self.state.boost_enabled = saved.boost_enabled;
                    self.state.smt_enabled = saved.smt_enabled;
                    self.state.core_preset = saved.core_preset;
                    self.fan_view_temp = fit_fan_view(&self.state.fan_curve.points);
                    self.set_status(&format!("Loaded '{}'.", self.selected_profile_name));
                }
                Task::none()
            }
            Message::ApplySavedProfile => {
                let Some(saved) = self
                    .saved_profiles
                    .iter()
                    .find(|p| p.name == self.selected_profile_name)
                    .cloned()
                else {
                    return Task::none();
                };
                let name = saved.name.clone();
                self.set_status(&format!("Applying '{name}'…"));
                spawn_blocking(
                    move || apply_full_profile(saved),
                    Message::SavedProfileApplied,
                )
            }
            Message::SavedProfileApplied(Ok(name)) => {
                profiles::save_active(&name);
                notify_daemon_profile_applied(&name);
                self.set_status(&format!("Applied '{name}'."));
                Task::none()
            }
            Message::SavedProfileApplied(Err(e)) => {
                self.set_status(&format!("Error: {e}"));
                Task::none()
            }
            Message::ReplaceProfile => {
                if self.selected_profile_name.is_empty() {
                    return Task::none();
                }
                let name = self.selected_profile_name.clone();
                let saved = SavedProfile {
                    name: name.clone(),
                    platform_profile: self.state.profile.clone(),
                    ppt: self.state.ppt.clone(),
                    fan_curve: self.state.fan_curve.points.clone(),
                    fan_hysteresis: self.state.fan_curve.hysteresis,
                    boost_enabled: self.state.boost_enabled,
                    smt_enabled: self.state.smt_enabled,
                    core_preset: self.state.core_preset.clone(),
                };
                profiles::upsert(&mut self.saved_profiles, saved);
                profiles::save(&self.saved_profiles);
                self.set_status(&format!("Replaced '{name}' with current state."));
                Task::none()
            }
            Message::DeleteProfile => {
                self.saved_profiles
                    .retain(|p| p.name != self.selected_profile_name);
                self.selected_profile_name = self
                    .saved_profiles
                    .first()
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                profiles::save(&self.saved_profiles);
                self.set_status("Profile deleted.");
                Task::none()
            }

        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let top = self.top_bar();
        let tabs = self.tab_strip();
        let content: Element<'_, Message> = match self.tab {
            Tab::Overview => views::overview::view(self),
            Tab::Profile => views::profile::view(self),
            Tab::Cooling => views::cooling::view(self),
            Tab::Saved => views::saved::view(self),
        };
        let body = container(content)
            .padding(20)
            .width(Length::Fill)
            .height(Length::Fill);
        let status = self.status_bar();

        container(column![top, tabs, body, status].spacing(0))
            .style(theme::root)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let title = row![
            container(Space::new(10, 10))
                .style(theme::pill(theme::MAUVE))
                .padding(0),
            text("strixctl").size(22).color(theme::TEXT),
            text("AMD power & cooling control").size(13).color(theme::SUBTEXT0),
        ]
        .spacing(10)
        .align_y(Alignment::Center);

        let mut right = row![].spacing(8).align_y(Alignment::Center);
        if self.state.current_temp > 0.0 {
            right = right.push(temp_pill("CPU", self.state.current_temp));
        }
        if let Some(g) = self.state.current_gpu_temp {
            right = right.push(temp_pill("GPU", g));
        }
        right = right.push(
            button(text("Reload").size(13))
                .on_press(Message::Reload)
                .style(theme::ghost_btn)
                .padding([6, 12]),
        );

        container(
            row![title, Space::with_width(Length::Fill), right]
                .align_y(Alignment::Center)
                .padding([14, 20]),
        )
        .style(theme::topbar)
        .width(Length::Fill)
        .into()
    }

    fn tab_strip(&self) -> Element<'_, Message> {
        let mut r = row![].spacing(4).padding([0, 16]);
        for t in Tab::ALL {
            let selected = self.tab == t;
            r = r.push(
                button(text(t.label()).size(14))
                    .on_press(Message::TabSelected(t))
                    .style(theme::tab_btn(selected))
                    .padding([10, 16]),
            );
        }
        container(r)
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::MANTLE)),
                ..Default::default()
            })
            .width(Length::Fill)
            .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let msg: Element<'_, Message> = if self.state.status_msg.is_empty() {
            text(" ").size(12).into()
        } else {
            text(self.state.status_msg.clone())
                .size(12)
                .color(theme::SUBTEXT1)
                .into()
        };
        container(
            row![
                msg,
                Space::with_width(Length::Fill),
                text(format!("v{}", env!("CARGO_PKG_VERSION")))
                    .size(11)
                    .color(theme::OVERLAY0),
            ]
            .align_y(Alignment::Center)
            .padding([6, 20]),
        )
        .style(theme::statusbar)
        .width(Length::Fill)
        .into()
    }

    pub fn set_status(&mut self, msg: &str) {
        self.state.status_msg = msg.to_string();
        self.status_token = self.status_token.wrapping_add(1);
    }

    fn set_ppt(&mut self, ppt: PptLimits) {
        self.ppt_apu_str = ppt.apu_limit.to_string();
        self.ppt_slow_str = ppt.slow_limit.to_string();
        self.ppt_fast_str = ppt.fast_limit.to_string();
        self.state.ppt = ppt;
    }

    fn start_ppt_reload(&mut self) -> Task<Message> {
        self.ppt_reload_inflight = true;
        Task::perform(
            async {
                tokio::time::sleep(Duration::from_millis(800)).await;
                tokio::task::spawn_blocking(backend::read_current_ppt)
                    .await
                    .unwrap_or(None)
            },
            Message::PptReloaded,
        )
    }
}

fn temp_pill<'a>(label: &'static str, t: f32) -> Element<'a, Message> {
    let color = theme::temp_level_color(t);
    container(
        text(format!("{}  {:.1}°C", label, t))
            .size(12)
            .color(color),
    )
    .style(theme::pill(color))
    .padding([4, 10])
    .into()
}

fn spawn_blocking<T>(
    f: impl FnOnce() -> T + Send + 'static,
    msg: fn(T) -> Message,
) -> Task<Message>
where
    T: Send + 'static,
{
    Task::perform(
        async move {
            tokio::task::spawn_blocking(f)
                .await
                .expect("blocking task panicked")
        },
        msg,
    )
}

fn apply_full_profile(saved: SavedProfile) -> Result<String, String> {
    let profile = saved.platform_profile.clone();
    backend::apply_profile(&profile)?;
    // Apply the fan curve before PPT: `asusctl fan-curve --enable-fan-curves
    // true` makes asusd reload its internal power state, which overwrites any
    // PPT registers ryzenadj set. Wait for asusd to settle, then write PPT —
    // mirroring the daemon's `run_apply_saved_profile`.
    backend::apply_fan_curve(
        &profile,
        &crate::state::FanCurve {
            points: saved.fan_curve.clone(),
            hysteresis: saved.fan_hysteresis,
        },
    )?;
    std::thread::sleep(std::time::Duration::from_millis(800));
    backend::apply_ppt(&saved.ppt)?;
    backend::set_boost(saved.boost_enabled)?;
    // SMT must be applied before the core preset, so the sibling-thread
    // sysfs entries exist (or don't) when `cores` writes to them.
    backend::set_smt(saved.smt_enabled)?;
    backend::set_core_preset(&saved.core_preset)?;
    Ok(saved.name)
}

fn notify_daemon_profile_applied(name: &str) {
    let _ = std::process::Command::new("gdbus")
        .args([
            "call",
            "--session",
            "--dest",
            "com.strixctl.Service",
            "--object-path",
            "/com/strixctl/Service",
            "--method",
            "com.strixctl.Service.NotifyProfileApplied",
            &format!("'{}'", name.replace('\'', "\\'")),
        ])
        .spawn();
}

pub fn fit_fan_view(points: &[(f32, f32)]) -> (f32, f32) {
    if points.is_empty() {
        return (0.0, 100.0);
    }
    let min_t = points.iter().map(|(t, _)| *t).fold(f32::INFINITY, f32::min);
    let max_t = points
        .iter()
        .map(|(t, _)| *t)
        .fold(f32::NEG_INFINITY, f32::max);
    let pad = ((max_t - min_t) * 0.15).max(5.0);
    (
        (min_t - pad).max(0.0),
        (max_t + pad).min(100.0),
    )
}
