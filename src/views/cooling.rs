use iced::widget::{button, column, container, row, text, text_input, Space};
use iced::{Element, Length};

use crate::app::{App, Message};
use crate::theme;
use crate::views::profile::card_section;
use crate::widgets::fan_curve;

pub fn view(app: &App) -> Element<'_, Message> {
    let header = row![
        text("Fan Curve").size(16).color(theme::TEXT),
        Space::with_width(Length::Fill),
        row![
            text("Hysteresis").size(12).color(theme::SUBTEXT0),
            text_input("", &app.hyst_str)
                .on_input(Message::HysteresisChanged)
                .style(theme::input)
                .padding([6, 8])
                .size(13)
                .width(Length::Fixed(56.0)),
            text("°C").size(12).color(theme::SUBTEXT0),
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center),
        button(text("Fit").size(13))
            .on_press(Message::FanFit)
            .style(theme::ghost_btn)
            .padding([6, 14]),
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center);

    let canvas = container(fan_curve::view(
        &app.state.fan_curve.points,
        app.state.current_temp,
        app.fan_view_temp,
    ))
    .style(|_| iced::widget::container::Style {
        background: Some(iced::Background::Color(theme::CRUST)),
        border: iced::Border {
            color: theme::SURFACE1,
            width: 1.0,
            radius: iced::border::Radius::from(8.0),
        },
        ..Default::default()
    })
    .width(Length::Fill)
    .height(Length::Fixed(340.0));

    let shift_row = row![
        button(text("−5°C").size(13))
            .on_press(Message::FanShift(-5))
            .style(theme::ghost_btn)
            .padding([6, 14]),
        button(text("+5°C").size(13))
            .on_press(Message::FanShift(5))
            .style(theme::ghost_btn)
            .padding([6, 14]),
        text("Custom").size(12).color(theme::SUBTEXT0),
        text_input("Δ°C", &app.shift_input)
            .on_input(Message::ShiftInputChanged)
            .style(theme::input)
            .padding([6, 8])
            .size(13)
            .width(Length::Fixed(64.0)),
        button(text("Shift").size(13))
            .on_press(Message::ApplyShift)
            .style(theme::ghost_btn)
            .padding([6, 14]),
        Space::with_width(Length::Fill),
        button(text("Apply Curve").size(13))
            .on_press(Message::ApplyFanCurve)
            .style(theme::primary_btn)
            .padding([8, 16]),
    ]
    .spacing(8)
    .align_y(iced::Alignment::Center);

    card_section(
        "Cooling",
        "Drag points to edit the per-profile fan curve. Applied for both CPU and GPU fans via asusctl.",
        column![header, Space::with_height(8), canvas, Space::with_height(14), shift_row]
            .spacing(0),
    )
}
