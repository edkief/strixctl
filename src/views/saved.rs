use iced::widget::{button, column, container, row, scrollable, text, text_input, Space};
use iced::{Element, Length};

use crate::app::{App, Message};
use crate::theme;
use crate::views::profile::card_section;

pub fn view(app: &App) -> Element<'_, Message> {
    let save_row = row![
        text_input("New profile name…", &app.new_profile_name)
            .on_input(Message::NewProfileNameChanged)
            .on_submit(Message::SaveProfile)
            .style(theme::input)
            .padding([8, 10])
            .size(14)
            .width(Length::Fill),
        {
            let mut b = button(text("Save Current State").size(13))
                .style(theme::primary_btn)
                .padding([8, 16]);
            if !app.new_profile_name.trim().is_empty() {
                b = b.on_press(Message::SaveProfile);
            }
            b
        },
    ]
    .spacing(10)
    .align_y(iced::Alignment::Center);

    let list: Element<'_, Message> = if app.saved_profiles.is_empty() {
        container(
            text("No saved profiles yet. Configure the system above, then save a snapshot here.")
                .size(13)
                .color(theme::SUBTEXT0),
        )
        .padding(20)
        .center_x(Length::Fill)
        .into()
    } else {
        let mut rows = column![].spacing(6);
        for p in &app.saved_profiles {
            let selected = p.name == app.selected_profile_name;
            rows = rows.push(row_for(p, selected));
        }
        scrollable(rows).height(Length::Fixed(320.0)).into()
    };

    card_section(
        "Saved Profiles",
        "Snapshot the current state and apply it later in one click.",
        column![save_row, Space::with_height(14), list].spacing(0),
    )
}

fn row_for<'a>(p: &'a crate::profiles::SavedProfile, selected: bool) -> Element<'a, Message> {
    let select_btn = button(
        row![
            text(&p.name).size(14).color(theme::TEXT),
            Space::with_width(Length::Fill),
            text(format!(
                "{}  ·  {}W APU  ·  boost {}",
                p.platform_profile.as_str(),
                p.ppt.apu_limit / 1000,
                if p.boost_enabled { "on" } else { "off" }
            ))
            .size(12)
            .color(theme::SUBTEXT0),
        ]
        .align_y(iced::Alignment::Center)
        .spacing(10),
    )
    .on_press(Message::SelectProfile(p.name.clone()))
    .style(theme::row_btn(selected))
    .padding([10, 14])
    .width(Length::Fill);

    let actions = row![
        button(text("Load").size(12))
            .on_press(Message::LoadProfile)
            .style(theme::ghost_btn)
            .padding([6, 12]),
        button(text("Apply").size(12))
            .on_press(Message::ApplySavedProfile)
            .style(theme::primary_btn)
            .padding([6, 12]),
        button(text("Replace").size(12))
            .on_press(Message::ReplaceProfile)
            .style(theme::ghost_btn)
            .padding([6, 12]),
        button(text("Delete").size(12))
            .on_press(Message::DeleteProfile)
            .style(theme::danger_btn)
            .padding([6, 12]),
    ]
    .spacing(6);

    if selected {
        row![select_btn, actions]
            .spacing(10)
            .align_y(iced::Alignment::Center)
            .into()
    } else {
        select_btn.into()
    }
}
