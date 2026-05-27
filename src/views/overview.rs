use iced::widget::{column, container, row, text, Space};
use iced::{Alignment, Color, Element, Length};

use crate::app::{App, Message};
use crate::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let profile_tile = kpi(
        "Current Profile",
        app.state.profile.as_str().to_string(),
        theme::MAUVE,
        "asusctl platform profile",
    );

    let cpu_value = if app.state.current_temp > 0.0 {
        format!("{:.1}°C", app.state.current_temp)
    } else {
        "—".to_string()
    };
    let cpu_tile = kpi(
        "CPU Temp",
        cpu_value,
        theme::temp_level_color(app.state.current_temp),
        "k10temp Tctl",
    );

    let gpu_value = app
        .state
        .current_gpu_temp
        .map(|t| format!("{:.1}°C", t))
        .unwrap_or_else(|| "—".to_string());
    let gpu_tile = kpi(
        "GPU Temp",
        gpu_value,
        theme::temp_level_color(app.state.current_gpu_temp.unwrap_or(0.0)),
        "amdgpu edge",
    );

    let watts = (app.state.ppt.apu_limit as f32) / 1000.0;
    let ppt_tile = kpi(
        "APU Power",
        format!("{:.0} W", watts),
        theme::BLUE,
        "STAPM sustain",
    );

    let tiles = row![profile_tile, cpu_tile, gpu_tile, ppt_tile]
        .spacing(16)
        .align_y(Alignment::Center);

    let cores = app.state.core_preset.as_u32();
    let threads = if app.state.smt_enabled { cores * 2 } else { cores };
    let preset_label = format!("{}C / {}T", cores, threads);
    let boost_label = if app.state.boost_enabled { "On" } else { "Off" };
    let smt_label = if app.state.smt_enabled { "On" } else { "Off" };

    let summary = container(
        column![
            text("System Summary").size(16).color(theme::TEXT),
            Space::with_height(8),
            summary_row("Platform profile", app.state.profile.as_str().to_string()),
            summary_row("CPU boost", boost_label.to_string()),
            summary_row("SMT (sibling threads)", smt_label.to_string()),
            summary_row("Active cores", preset_label),
            summary_row(
                "Power limits (mW)",
                format!(
                    "APU {}  ·  slow {}  ·  fast {}",
                    app.state.ppt.apu_limit,
                    app.state.ppt.slow_limit,
                    app.state.ppt.fast_limit
                ),
            ),
            summary_row(
                "Fan curve points",
                format!("{} points", app.state.fan_curve.points.len()),
            ),
        ]
        .spacing(6)
        .padding(20),
    )
    .style(theme::card)
    .width(Length::Fill);

    column![tiles, summary].spacing(20).into()
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
