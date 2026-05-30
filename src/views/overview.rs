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
    if platform::SUPPORTS_CORE_PRESET {
        rows.push(summary_row("Active cores", preset_label));
        if platform::CORE_PRESET_NEEDS_REBOOT && app.state.core_reboot_pending {
            rows.push(reboot_banner());
        }
    }
    if platform::SUPPORTS_PPT {
        rows.push(summary_row(
            "Power limits (mW)",
            format!(
                "APU {}  ·  slow {}  ·  fast {}",
                app.state.ppt.apu_limit, app.state.ppt.slow_limit, app.state.ppt.fast_limit
            ),
        ));
    }
    if platform::SUPPORTS_FAN_CURVE {
        rows.push(summary_row(
            "Fan curve points",
            format!("{} points", app.state.fan_curve.points.len()),
        ));
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
