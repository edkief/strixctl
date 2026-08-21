use iced::widget::{column, container, row, text, Column, Row, Space};
use iced::{Alignment, Color, Element, Length};

use crate::app::{App, Message};
use crate::platform;
use crate::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    // ----- KPI tiles (only the ones the platform can populate) -----
    let mut tiles: Vec<Element<Message>> = Vec::new();

    if platform::SUPPORTS_PLATFORM_PROFILE {
        let subtitle = if cfg!(windows) {
            "atrofac power plan"
        } else {
            "asusctl platform profile"
        };
        tiles.push(kpi(
            "Current Profile",
            app.state.profile.as_str().to_string(),
            theme::MAUVE,
            subtitle,
        ));
    }

    if platform::SUPPORTS_TEMP {
        let cpu_value = if app.state.current_temp > 0.0 {
            format!("{:.1}°C", app.state.current_temp)
        } else {
            "—".to_string()
        };
        tiles.push(kpi(
            "CPU Temp",
            cpu_value,
            theme::temp_level_color(app.state.current_temp),
            "k10temp Tctl",
        ));

        let gpu_value = app
            .state
            .current_gpu_temp
            .map(|t| format!("{:.1}°C", t))
            .unwrap_or_else(|| "—".to_string());
        tiles.push(kpi(
            "GPU Temp",
            gpu_value,
            theme::temp_level_color(app.state.current_gpu_temp.unwrap_or(0.0)),
            "amdgpu edge",
        ));
    }

    if platform::SUPPORTS_FREQ_DISPLAY {
        let value = app
            .state
            .current_cpu_freq_mhz
            .map(|mhz| format!("{:.2} GHz", mhz as f32 / 1000.0))
            .unwrap_or_else(|| "—".to_string());
        let hint = match app.state.max_freq_khz {
            Some(khz) => format!("fastest core · capped {} MHz", khz / 1000),
            None => "fastest core · uncapped".to_string(),
        };
        tiles.push(kpi_owned("CPU Freq", value, theme::SKY, hint));
    }

    if platform::SUPPORTS_PPT {
        let watts = (app.state.ppt.apu_limit as f32) / 1000.0;
        tiles.push(kpi(
            "APU Power",
            format!("{:.0} W", watts),
            theme::BLUE,
            "STAPM sustain",
        ));
    }

    let tiles = Row::with_children(tiles)
        .spacing(16)
        .align_y(Alignment::Center);

    // ----- System summary (rows gated to supported features) -----
    let cores = app.state.core_preset.as_u32();
    let threads = if app.state.smt_enabled { cores * 2 } else { cores };
    let preset_label = format!("{}C / {}T", cores, threads);

    let mut rows: Vec<Element<Message>> = Vec::new();
    rows.push(text("System Summary").size(16).color(theme::TEXT).into());
    rows.push(Space::with_height(8).into());

    if platform::SUPPORTS_PLATFORM_PROFILE {
        rows.push(summary_row("Platform profile", app.state.profile.as_str().to_string()));
    }
    if platform::SUPPORTS_BOOST {
        let boost_label = if app.state.boost_enabled { "On" } else { "Off" };
        rows.push(summary_row("CPU boost", boost_label.to_string()));
    }
    if platform::SUPPORTS_SMT {
        let smt_label = if app.state.smt_enabled { "On" } else { "Off" };
        rows.push(summary_row("SMT (sibling threads)", smt_label.to_string()));
    }
    if platform::SUPPORTS_FREQ_DISPLAY || platform::SUPPORTS_MAX_FREQ {
        let mut badges = Vec::new();
        if platform::SUPPORTS_FREQ_DISPLAY {
            let (label, color) = match app.state.current_cpu_freq_mhz {
                Some(mhz) => (format!("now  {} MHz", mhz), theme::SKY),
                None => ("now  —".to_string(), theme::OVERLAY1),
            };
            badges.push(badge(label, color));
        }
        if platform::SUPPORTS_MAX_FREQ {
            let (label, color) = match app.state.max_freq_khz {
                Some(khz) => (format!("cap  {} MHz", khz / 1000), theme::PEACH),
                None => ("cap  off".to_string(), theme::OVERLAY1),
            };
            badges.push(badge(label, color));
        }
        rows.push(summary_row_badges("CPU frequency", badges));
    }
    if platform::SUPPORTS_CORE_PRESET {
        rows.push(summary_row("Active cores", preset_label));
        if platform::CORE_PRESET_NEEDS_REBOOT && app.state.core_reboot_pending {
            rows.push(reboot_banner());
        }
    }
    if platform::SUPPORTS_PPT {
        let ppt = &app.state.ppt;
        rows.push(summary_row_badges(
            "Power limits",
            vec![
                badge(format!("APU  {:.0} W", ppt.apu_limit as f32 / 1000.0), theme::BLUE),
                badge(format!("Slow  {:.0} W", ppt.slow_limit as f32 / 1000.0), theme::BLUE),
                badge(format!("Fast  {:.0} W", ppt.fast_limit as f32 / 1000.0), theme::BLUE),
            ],
        ));
    }
    if platform::SUPPORTS_FAN_CURVE {
        let pts = &app.state.fan_curve.points;
        let badges = if pts.is_empty() {
            vec![badge("—".to_string(), theme::OVERLAY1)]
        } else {
            let (t0, f0) = pts.first().unwrap();
            let (t1, f1) = pts.last().unwrap();
            if pts.len() == 1 {
                vec![badge(format!("{:.0} C  {:.0}%", t0, f0), theme::TEAL)]
            } else {
                vec![
                    badge(format!("{:.0} C  {:.0}%", t0, f0), theme::TEAL),
                    badge(format!("{:.0} C  {:.0}%", t1, f1), theme::TEAL),
                ]
            }
        };
        rows.push(summary_row_badges("Fan curve", badges));
    }

    let summary = container(Column::with_children(rows).spacing(6).padding(20))
        .style(theme::card)
        .width(Length::Fill);

    column![tiles, summary].spacing(20).into()
}

fn reboot_banner<'a>() -> Element<'a, Message> {
    container(
        text("⚠  Core-count change pending — restart Windows to apply.")
            .size(12)
            .color(theme::BASE),
    )
    .style(theme::pill(theme::PEACH))
    .padding([6, 12])
    .into()
}

/// Same tile as `kpi`, but for hints built at call time rather than static text.
fn kpi_owned<'a>(label: &'a str, value: String, accent: Color, hint: String) -> Element<'a, Message> {
    container(
        column![
            text(label).size(12).color(theme::SUBTEXT0),
            Space::with_height(4),
            text(value).size(28).color(accent),
            Space::with_height(2),
            text(hint).size(11).color(theme::OVERLAY0),
        ]
        .spacing(0)
        .padding(18),
    )
    .style(theme::kpi_tile)
    .width(Length::Fill)
    .into()
}

fn kpi<'a>(label: &'a str, value: String, accent: Color, hint: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(label).size(12).color(theme::SUBTEXT0),
            Space::with_height(4),
            text(value).size(28).color(accent),
            Space::with_height(2),
            text(hint).size(11).color(theme::OVERLAY0),
        ]
        .spacing(0)
        .padding(18),
    )
    .style(theme::kpi_tile)
    .width(Length::Fill)
    .into()
}

fn summary_row<'a>(k: &'a str, v: String) -> Element<'a, Message> {
    row![
        text(k).size(13).color(theme::SUBTEXT1).width(Length::FillPortion(1)),
        text(v).size(13).color(theme::TEXT).width(Length::FillPortion(2)),
    ]
    .align_y(Alignment::Center)
    .into()
}

fn summary_row_badges<'a>(k: &'a str, badges: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(k).size(13).color(theme::SUBTEXT1).width(Length::FillPortion(1)),
        Row::with_children(badges).spacing(6).width(Length::FillPortion(2)),
    ]
    .align_y(Alignment::Center)
    .into()
}

fn badge<'a>(label: impl ToString, color: iced::Color) -> Element<'a, Message> {
    container(text(label.to_string()).size(12).color(color))
        .style(theme::pill(color))
        .padding([3, 10])
        .into()
}
