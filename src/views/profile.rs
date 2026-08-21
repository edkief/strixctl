use iced::widget::{button, column, container, row, text, text_input, toggler, Column, Space};
use iced::{Alignment, Element, Length};

use crate::app::{App, Message};
use crate::platform;
use crate::state::{CorePreset, Profile};
use crate::theme;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut cards: Vec<Element<Message>> = Vec::new();
    if platform::SUPPORTS_PLATFORM_PROFILE {
        cards.push(profile_card(app));
    }
    if platform::SUPPORTS_PPT {
        cards.push(ppt_card(app));
    }
    cards.push(cpu_card(app));
    Column::with_children(cards).spacing(20).into()
}

// ---------- Platform Profile ----------

fn profile_card(app: &App) -> Element<'_, Message> {
    let hint = if cfg!(windows) {
        "Switches the atrofac power plan (Quiet → silent, Balanced → windows, Performance → turbo)."
    } else {
        "Switches asusctl's power profile (Quiet, Balanced, Performance)."
    };
    card_section(
        "Platform Profile",
        hint,
        row![
            profile_seg("Quiet", Profile::Quiet, &app.state.profile),
            profile_seg("Balanced", Profile::Balanced, &app.state.profile),
            profile_seg("Performance", Profile::Performance, &app.state.profile),
            Space::with_width(Length::Fill),
            button(text("Apply Profile").size(13))
                .on_press(Message::ApplyProfile)
                .style(theme::primary_btn)
                .padding([8, 16]),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
}

fn profile_seg<'a>(label: &'a str, value: Profile, current: &Profile) -> Element<'a, Message> {
    let selected = current == &value;
    button(text(label).size(14))
        .on_press(Message::SetProfile(value))
        .style(theme::segment_btn(selected))
        .padding([10, 20])
        .into()
}

// ---------- Power Limits ----------

fn ppt_card(app: &App) -> Element<'_, Message> {
    let valid = parse_valid(app);
    let mut apply = button(text("Apply Power Limits").size(13))
        .style(theme::primary_btn)
        .padding([8, 16]);
    if valid && !app.ppt_reload_inflight {
        apply = apply.on_press(Message::ApplyPpt);
    }

    let hint = if cfg!(windows) {
        "ryzenadj STAPM / slow / fast PPT — requires Administrator (UAC prompt)."
    } else {
        "ryzenadj STAPM / slow / fast PPT — requires pkexec authorization."
    };

    card_section(
        "Power Limits (mW)",
        hint,
        column![
            row![
                labelled_input("APU / STAPM", &app.ppt_apu_str, Message::PptApuChanged),
                labelled_input("Slow", &app.ppt_slow_str, Message::PptSlowChanged),
                labelled_input("Fast", &app.ppt_fast_str, Message::PptFastChanged),
                column![Space::with_height(18), apply].spacing(0),
            ]
            .spacing(14)
            .align_y(Alignment::Start),
            Space::with_height(8),
            validation_msg(app),
        ]
        .spacing(0),
    )
}

fn labelled_input<'a>(
    label: &'a str,
    value: &str,
    on_change: impl Fn(String) -> Message + 'a,
) -> Element<'a, Message> {
    column![
        text(label).size(12).color(theme::SUBTEXT0),
        Space::with_height(4),
        text_input("", value)
            .on_input(on_change)
            .style(theme::input)
            .padding([8, 10])
            .size(14)
            .width(Length::Fill),
    ]
    .width(Length::Fill)
    .into()
}

fn validation_msg(app: &App) -> Element<'_, Message> {
    if let (Ok(sl), Ok(f)) = (
        app.ppt_slow_str.parse::<u32>(),
        app.ppt_fast_str.parse::<u32>(),
    ) {
        if sl > f {
            return text("⚠  Slow limit must not exceed Fast limit")
                .size(12)
                .color(theme::PEACH)
                .into();
        }
    }
    let any_invalid = app.ppt_apu_str.parse::<u32>().is_err()
        || app.ppt_slow_str.parse::<u32>().is_err()
        || app.ppt_fast_str.parse::<u32>().is_err();
    if any_invalid {
        text("Enter integer milliwatts in each field.")
            .size(12)
            .color(theme::OVERLAY1)
            .into()
    } else {
        text(" ").size(12).into()
    }
}

fn parse_valid(app: &App) -> bool {
    match (
        app.ppt_apu_str.parse::<u32>(),
        app.ppt_slow_str.parse::<u32>(),
        app.ppt_fast_str.parse::<u32>(),
    ) {
        (Ok(_), Ok(sl), Ok(f)) => sl <= f,
        _ => false,
    }
}

// ---------- CPU ----------
//
// Rewritten from scratch: every row uses the same skeleton —
//   [ label column (Fill) ] [ segmented control ] [ Apply ]
// No iced `toggler` anywhere; on/off and presets are all rendered as
// styled buttons we already use elsewhere, so the layout is bulletproof.

fn cpu_card(app: &App) -> Element<'_, Message> {
    let smt = app.state.smt_enabled;
    let cores = app.state.core_preset.as_u32();
    let threads = if smt { cores * 2 } else { cores };

    let mut items: Vec<Element<Message>> = Vec::new();

    if platform::SUPPORTS_BOOST {
        items.push(control_row(
            "CPU Boost",
            "Sets /sys/devices/system/cpu/cpufreq/boost.",
            toggler(app.state.boost_enabled)
                .on_toggle(Message::BoostToggled)
                .size(22)
                .into(),
            Message::ApplyBoost,
        ));
    }
    if platform::SUPPORTS_SMT {
        if !items.is_empty() {
            items.push(divider());
        }
        items.push(control_row(
            "SMT",
            "Simultaneous multi-threading — sibling threads on or off.",
            toggler(smt).on_toggle(Message::SmtToggled).size(22).into(),
            Message::ApplySmt,
        ));
    }
    if platform::SUPPORTS_MAX_FREQ {
        if !items.is_empty() {
            items.push(divider());
        }
        items.push(max_freq_row(app));
    }
    if platform::SUPPORTS_CORE_PRESET {
        if !items.is_empty() {
            items.push(divider());
        }
        let cores_hint = if platform::CORE_PRESET_NEEDS_REBOOT {
            format!("Currently {cores}C / {threads}T. Applied via bcdedit — takes effect after a reboot.")
        } else {
            format!("Currently {cores}C / {threads}T.")
        };
        items.push(control_row(
            "Active Cores",
            &cores_hint,
            core_preset_segment(&app.state.core_preset, smt),
            Message::ApplyCorePreset,
        ));
        if platform::CORE_PRESET_NEEDS_REBOOT && app.state.core_reboot_pending {
            items.push(reboot_banner());
        }
    }

    let card_hint = if platform::SUPPORTS_BOOST {
        "Boost, SMT, frequency cap, and core-count controls. Apply SMT before \
         changing the core preset — sibling threads vanish from sysfs when SMT \
         is off."
    } else {
        "Frequency cap (powercfg) and active core count (bcdedit, requires a reboot)."
    };

    card_section("CPU", card_hint, Column::with_children(items).spacing(0))
}

/// Max-frequency cap: a toggler that engages the cap plus an MHz input.
///
/// The whole row is inert when the platform can read a frequency range but
/// didn't find one (no cpufreq sysfs) — the control stays visible and greyed so
/// it's clear the feature exists but this machine can't do it.
fn max_freq_row(app: &App) -> Element<'_, Message> {
    let range = app.state.freq_range_khz;
    let unavailable = platform::MAX_FREQ_READBACK && range.is_none();
    let capped = app.max_freq_capped;

    let hint = if unavailable {
        "Unavailable — this system exposes no cpufreq scaling range.".to_string()
    } else if let Some((lo, hi)) = range {
        match app.state.max_freq_khz {
            Some(k) => format!(
                "Capped at {} MHz (hardware {}–{} MHz). Applied with cpupower / scaling_max_freq.",
                k / 1000,
                lo / 1000,
                hi / 1000
            ),
            None => format!(
                "Uncapped (hardware {}–{} MHz). Applied with cpupower / scaling_max_freq.",
                lo / 1000,
                hi / 1000
            ),
        }
    } else {
        "Sets the power plan's maximum processor frequency (powercfg PROCFREQMAX)."
            .to_string()
    };

    let mut toggle = toggler(capped).size(22);
    let mut input = text_input("MHz", &app.max_freq_str)
        .style(theme::input)
        .padding([6, 10])
        .size(14)
        .width(Length::Fixed(90.0));
    let mut apply = button(text("Apply").size(13))
        .style(theme::ghost_btn)
        .padding([6, 14]);

    if !unavailable {
        toggle = toggle.on_toggle(Message::MaxFreqCapToggled);
        apply = apply.on_press(Message::ApplyMaxFreq);
        if capped {
            input = input.on_input(Message::MaxFreqChanged);
        }
    }

    row![
        column![
            text("Max CPU Frequency").size(14).color(theme::TEXT),
            text(hint).size(11).color(theme::OVERLAY1),
        ]
        .spacing(2)
        .width(Length::Fill),
        toggle,
        input,
        text("MHz").size(12).color(theme::SUBTEXT0),
        apply,
    ]
    .align_y(Alignment::Center)
    .spacing(10)
    .into()
}

fn reboot_banner<'a>() -> Element<'a, Message> {
    column![
        Space::with_height(12),
        container(
            text("⚠  Core-count change pending — restart Windows to apply.")
                .size(12)
                .color(theme::BASE),
        )
        .style(theme::pill(theme::PEACH))
        .padding([6, 12]),
    ]
    .into()
}

fn control_row<'a>(
    title: &'a str,
    hint: &str,
    control: Element<'a, Message>,
    on_apply: Message,
) -> Element<'a, Message> {
    row![
        column![
            text(title).size(14).color(theme::TEXT),
            text(hint.to_string()).size(11).color(theme::OVERLAY1),
        ]
        .spacing(2)
        .width(Length::Fill),
        control,
        button(text("Apply").size(13))
            .on_press(on_apply)
            .style(theme::ghost_btn)
            .padding([6, 14]),
    ]
    .align_y(Alignment::Center)
    .spacing(14)
    .into()
}

fn core_preset_segment<'a>(current: &CorePreset, smt: bool) -> Element<'a, Message> {
    let mk = |label_top: &'static str, threads_smt: u32, threads_off: u32, val: CorePreset| {
        let selected = current == &val;
        let (top, sub) = if selected {
            (theme::BASE, iced::Color { a: 0.7, ..theme::BASE })
        } else {
            (theme::TEXT, theme::SUBTEXT0)
        };
        let threads = if smt { threads_smt } else { threads_off };
        button(
            column![
                text(label_top).size(13).color(top),
                text(format!("{}T", threads)).size(10).color(sub),
            ]
            .align_x(Alignment::Center)
            .spacing(1),
        )
        .on_press(Message::SetCorePreset(val))
        .style(theme::segment_btn(selected))
        .padding([6, 14])
    };

    row![
        mk("4C", 8, 4, CorePreset::Four),
        mk("8C", 16, 8, CorePreset::Eight),
        mk("12C", 24, 12, CorePreset::Twelve),
        mk("16C", 32, 16, CorePreset::Sixteen),
    ]
    .spacing(4)
    .into()
}

fn divider<'a>() -> Element<'a, Message> {
    column![
        Space::with_height(12),
        container(Space::with_height(1))
            .style(|_| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::SURFACE1)),
                ..Default::default()
            })
            .width(Length::Fill),
        Space::with_height(12),
    ]
    .into()
}

// ---------- Shared card frame ----------

pub fn card_section<'a>(
    title: &'a str,
    hint: &'a str,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    container(
        column![
            text(title).size(16).color(theme::TEXT),
            text(hint).size(12).color(theme::SUBTEXT0),
            Space::with_height(12),
            body.into(),
        ]
        .spacing(2)
        .padding(20),
    )
    .style(theme::card)
    .width(Length::Fill)
    .align_x(Alignment::Start)
    .into()
}
