use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Theme};

/// Zed's typefaces, IBM Plex Sans and Plex Mono, bundled from `assets/fonts` (SIL Open
/// Font License; see `assets/fonts/LICENSE.txt`).
pub const SANS: Font = Font::with_name("IBM Plex Sans");
pub const MONO: Font = Font::with_name("IBM Plex Mono");

// Colours sampled from Zed's dark theme (see the reference screenshot): near-black
// editor and thread columns, a slightly lighter sidebar, and light borders.
pub const EDITOR_BG: Color = Color::from_rgb8(0x19, 0x1c, 0x1f);
/// Sidebar.
pub const PANEL_BG: Color = Color::from_rgb8(0x20, 0x23, 0x27);
/// Footer, tooltips and menus.
pub const BAR_BG: Color = Color::from_rgb8(0x19, 0x1c, 0x1f);
/// Middle column that hosts threads, terminals and agents.
pub const AGENT_BG: Color = Color::from_rgb8(0x19, 0x1c, 0x1f);
/// Diff and code cards.
pub const CARD_BG: Color = Color::from_rgb8(0x16, 0x19, 0x1b);
pub const TAB_BAR_BG: Color = Color::from_rgb8(0x21, 0x24, 0x2a);
pub const BORDER: Color = Color::from_rgb8(0x2a, 0x2c, 0x32);
pub const HOVER: Color = Color::from_rgb8(0x26, 0x29, 0x2e);
pub const SELECTED: Color = Color::from_rgb8(0x2d, 0x2f, 0x35);
pub const TEXT: Color = Color::from_rgb8(0xdd, 0xe0, 0xe5);
pub const TEXT_DIM: Color = Color::from_rgb8(0xaa, 0xaf, 0xbc);
pub const TEXT_FAINT: Color = Color::from_rgb8(0x6b, 0x71, 0x7d);
pub const GUTTER: Color = Color::from_rgb8(0x5d, 0x63, 0x70);
pub const ACCENT: Color = Color::from_rgb8(0x74, 0xad, 0xe8);
pub const ADDED: Color = Color::from_rgb8(0xa1, 0xc1, 0x81);
pub const REMOVED: Color = Color::from_rgb8(0xd0, 0x72, 0x77);
/// Row backgrounds for added and removed lines.
pub const ADDED_BG: Color = Color::from_rgb8(0x20, 0x2e, 0x25);
pub const REMOVED_BG: Color = Color::from_rgb8(0x36, 0x1e, 0x23);
/// Comments: their bar, and (as `COMMENT_BG`) the highlight on quoted words.
pub const COMMENT: Color = Color::from_rgb8(0xb5, 0x8b, 0xe2);
pub const COMMENT_BG: Color = Color::from_rgb8(0x38, 0x32, 0x46);

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
                radius: 3.0.into(),
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
                radius: 3.0.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }
}

/// Single-line text box.
/// A text box with no frame, for typing straight into a feed.
pub fn bare(_: &Theme, _: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: Background::Color(Color::TRANSPARENT),
        border: Border { radius: 0.0.into(), width: 0.0, color: Color::TRANSPARENT },
        icon: TEXT_DIM,
        placeholder: TEXT_FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(EDITOR_BG),
        border: Border {
            radius: 3.0.into(),
            width: 1.0,
            color: if focused { ACCENT } else { SELECTED },
        },
        icon: TEXT_DIM,
        placeholder: TEXT_FAINT,
        value: TEXT,
        selection: Color { a: 0.35, ..ACCENT },
    }
}