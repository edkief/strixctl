// Catppuccin Mocha palette + reusable iced styles.
// Hex values are canonical from https://catppuccin.com/palette.

use iced::{
    Background, Border, Color, Shadow, Theme,
    border::Radius,
    widget::{button, container, text_input},
};

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::from_rgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
}

pub const ROSEWATER: Color = rgb(0xf5, 0xe0, 0xdc);
pub const MAUVE: Color = rgb(0xcb, 0xa6, 0xf7);
pub const RED: Color = rgb(0xf3, 0x8b, 0xa8);
pub const PEACH: Color = rgb(0xfa, 0xb3, 0x87);
pub const YELLOW: Color = rgb(0xf9, 0xe2, 0xaf);
pub const GREEN: Color = rgb(0xa6, 0xe3, 0xa1);
pub const TEAL: Color = rgb(0x94, 0xe2, 0xd5);
pub const SKY: Color = rgb(0x89, 0xdc, 0xeb);
pub const BLUE: Color = rgb(0x89, 0xb4, 0xfa);
pub const LAVENDER: Color = rgb(0xb4, 0xbe, 0xfe);

pub const TEXT: Color = rgb(0xcd, 0xd6, 0xf4);
pub const SUBTEXT1: Color = rgb(0xba, 0xc2, 0xde);
pub const SUBTEXT0: Color = rgb(0xa6, 0xad, 0xc8);
pub const OVERLAY2: Color = rgb(0x93, 0x99, 0xb2);
pub const OVERLAY1: Color = rgb(0x7f, 0x84, 0x9c);
pub const OVERLAY0: Color = rgb(0x6c, 0x70, 0x86);

pub const SURFACE2: Color = rgb(0x58, 0x5b, 0x70);
pub const SURFACE1: Color = rgb(0x45, 0x47, 0x5a);
pub const SURFACE0: Color = rgb(0x31, 0x32, 0x44);
pub const BASE: Color = rgb(0x1e, 0x1e, 0x2e);
pub const MANTLE: Color = rgb(0x18, 0x18, 0x25);
pub const CRUST: Color = rgb(0x11, 0x11, 0x1b);

pub fn mocha() -> Theme {
    Theme::custom(
        "Catppuccin Mocha".to_string(),
        iced::theme::Palette {
            background: BASE,
            text: TEXT,
            primary: MAUVE,
            success: GREEN,
            danger: RED,
        },
    )
}

// ---------- Containers ----------

pub fn root(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BASE)),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn topbar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(MANTLE)),
        text_color: Some(TEXT),
        border: Border {
            color: SURFACE0,
            width: 0.0,
            radius: Radius::from(0.0),
        },
        ..container::Style::default()
    }
}

pub fn statusbar(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(MANTLE)),
        text_color: Some(SUBTEXT0),
        ..container::Style::default()
    }
}

pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE0)),
        text_color: Some(TEXT),
        border: Border {
            color: SURFACE1,
            width: 1.0,
            radius: Radius::from(10.0),
        },
        shadow: Shadow::default(),
    }
}

pub fn kpi_tile(_: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(SURFACE0)),
        text_color: Some(TEXT),
        border: Border {
            color: SURFACE1,
            width: 1.0,
            radius: Radius::from(12.0),
        },
        shadow: Shadow::default(),
    }
}

pub fn pill(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(with_alpha(color, 0.18))),
        text_color: Some(color),
        border: Border {
            color: with_alpha(color, 0.35),
            width: 1.0,
            radius: Radius::from(999.0),
        },
        shadow: Shadow::default(),
    }
}

// ---------- Buttons ----------

pub fn primary_btn(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => mix(MAUVE, TEXT, 0.10),
        button::Status::Pressed => mix(MAUVE, BASE, 0.15),
        button::Status::Disabled => with_alpha(MAUVE, 0.35),
        _ => MAUVE,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: BASE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
    }
}

pub fn ghost_btn(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => SURFACE1,
        button::Status::Pressed => SURFACE2,
        _ => SURFACE0,
    };
    let text = match status {
        button::Status::Disabled => OVERLAY0,
        _ => TEXT,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: text,
        border: Border {
            color: SURFACE1,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
    }
}

pub fn danger_btn(_: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => mix(RED, TEXT, 0.10),
        button::Status::Pressed => mix(RED, BASE, 0.15),
        button::Status::Disabled => with_alpha(RED, 0.35),
        _ => RED,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: BASE,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: Radius::from(8.0),
        },
        shadow: Shadow::default(),
    }
}

pub fn segment_btn(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let (bg, text, border) = if selected {
            (MAUVE, BASE, MAUVE)
        } else {
            let bg = match status {
                button::Status::Hovered => SURFACE1,
                button::Status::Pressed => SURFACE2,
                _ => SURFACE0,
            };
            (bg, TEXT, SURFACE1)
        };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: text,
            border: Border {
                color: border,
                width: 1.0,
                radius: Radius::from(8.0),
            },
            shadow: Shadow::default(),
        }
    }
}

pub fn row_btn(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if selected {
            with_alpha(MAUVE, 0.18)
        } else {
            match status {
                button::Status::Hovered => SURFACE1,
                button::Status::Pressed => SURFACE2,
                _ => SURFACE0,
            }
        };
        let border = if selected { MAUVE } else { SURFACE1 };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: TEXT,
            border: Border {
                color: border,
                width: 1.0,
                radius: Radius::from(8.0),
            },
            shadow: Shadow::default(),
        }
    }
}

pub fn tab_btn(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let bg = if selected {
            BASE
        } else {
            match status {
                button::Status::Hovered => SURFACE0,
                _ => MANTLE,
            }
        };
        let text = if selected { MAUVE } else { SUBTEXT1 };
        let border_color = if selected { MAUVE } else { Color::TRANSPARENT };
        button::Style {
            background: Some(Background::Color(bg)),
            text_color: text,
            border: Border {
                color: border_color,
                width: if selected { 2.0 } else { 0.0 },
                radius: Radius {
                top_left: 8.0,
                top_right: 8.0,
                bottom_right: 0.0,
                bottom_left: 0.0,
            },
            },
            shadow: Shadow::default(),
        }
    }
}

// ---------- Inputs ----------

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let border_color = match status {
        text_input::Status::Focused { .. } => MAUVE,
        text_input::Status::Hovered => SURFACE2,
        text_input::Status::Disabled => SURFACE0,
        _ => SURFACE1,
    };
    text_input::Style {
        background: Background::Color(MANTLE),
        border: Border {
            color: border_color,
            width: 1.0,
            radius: Radius::from(8.0),
        },
        icon: SUBTEXT0,
        placeholder: OVERLAY0,
        value: TEXT,
        selection: with_alpha(MAUVE, 0.35),
    }
}

// ---------- Helpers ----------

pub fn temp_level_color(temp: f32) -> Color {
    if temp > 90.0 {
        RED
    } else if temp > 75.0 {
        PEACH
    } else {
        GREEN
    }
}

fn with_alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color {
        r: a.r * (1.0 - t) + b.r * t,
        g: a.g * (1.0 - t) + b.g * t,
        b: a.b * (1.0 - t) + b.b * t,
        a: a.a * (1.0 - t) + b.a * t,
    }
}
