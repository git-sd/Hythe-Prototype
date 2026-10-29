use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Theme};

// A One Dark-style palette in the spirit of Zed's default theme.
pub const EDITOR_BG: Color = Color::from_rgb8(0x28, 0x2c, 0x34);
/// Sidebar, title bar and bottom bar.
pub const PANEL_BG: Color = Color::from_rgb8(0x21, 0x25, 0x2b);
pub const BAR_BG: Color = Color::from_rgb8(0x21, 0x25, 0x2b);
/// Middle column that hosts terminals and agents.
pub const AGENT_BG: Color = Color::from_rgb8(0x26, 0x2a, 0x32);
pub const TAB_BAR_BG: Color = Color::from_rgb8(0x2b, 0x30, 0x38);
pub const BORDER: Color = Color::from_rgb8(0x18, 0x1a, 0x1f);
pub const HOVER: Color = Color::from_rgb8(0x2c, 0x31, 0x3a);
pub const SELECTED: Color = Color::from_rgb8(0x2f, 0x34, 0x3e);
pub const TEXT: Color = Color::from_rgb8(0xdc, 0xe0, 0xe5);
pub const TEXT_DIM: Color = Color::from_rgb8(0x8a, 0x91, 0x9e);
pub const TEXT_FAINT: Color = Color::from_rgb8(0x5d, 0x63, 0x6f);
pub const GUTTER: Color = Color::from_rgb8(0x4b, 0x52, 0x63);
pub const ACCENT: Color = Color::from_rgb8(0x74, 0xad, 0xe8);
pub const ADDED: Color = Color::from_rgb8(0xa1, 0xc1, 0x81);
pub const REMOVED: Color = Color::from_rgb8(0xd0, 0x72, 0x77);

pub fn bg(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(Background::Color(color)),
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
                radius: 5.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Like [`flat`] but the label stays bright, for rows in the sidebar.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (selected, status) {
            (true, _) => Some(SELECTED),
            (_, button::Status::Hovered | button::Status::Pressed) => Some(HOVER),
            _ => None,
        };
        button::Style {
            background: background.map(Background::Color),
            text_color: TEXT,
            border: Border {
                radius: 5.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Single-line text box.
pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(EDITOR_BG),
        border: Border {
            radius: 6.0.into(),
            width: 1.0,
            color: if focused { ACCENT } else { SELECTED },
        },
        icon: TEXT_DIM,
        placeholder: TEXT_FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}