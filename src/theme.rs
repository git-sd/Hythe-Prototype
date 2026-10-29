use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};

// Graphite surfaces with a teal accent.
pub const EDITOR_BG: Color = Color::from_rgb8(0x17, 0x1a, 0x1f);
pub const PANEL_BG: Color = Color::from_rgb8(0x1c, 0x20, 0x26);
pub const BAR_BG: Color = Color::from_rgb8(0x13, 0x15, 0x19);
pub const BORDER: Color = Color::from_rgb8(0x2a, 0x2f, 0x38);
pub const HOVER: Color = Color::from_rgb8(0x26, 0x2b, 0x33);
pub const SELECTED: Color = Color::from_rgb8(0x2c, 0x33, 0x3d);
pub const TEXT: Color = Color::from_rgb8(0xd5, 0xd9, 0xe0);
pub const TEXT_DIM: Color = Color::from_rgb8(0x8a, 0x92, 0xa0);
pub const ACCENT: Color = Color::from_rgb8(0x3f, 0xc1, 0xb0);

pub fn bg(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
        ..Default::default()
    }
}

/// Outlined, search-box-like button.
pub fn pill(_: &Theme, status: button::Status) -> button::Style {
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if hovered { HOVER } else { EDITOR_BG })),
        text_color: TEXT_DIM,
        border: Border {
            color: if hovered { ACCENT } else { BORDER },
            width: 1.0,
            radius: 7.0.into(),
        },
        ..Default::default()
    }
}

pub fn flat(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (selected, status) {
            (true, _) => Some(SELECTED),
            (_, button::Status::Hovered | button::Status::Pressed) => Some(HOVER),
            _ => None,
        };
        button::Style {
            background: background.map(Background::Color),
            text_color: if selected { TEXT } else { TEXT_DIM },
            border: Border {
                radius: 4.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}
