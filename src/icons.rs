//! Small line icons, drawn as SVG and tinted to any colour.

use iced::widget::svg;
use iced::{Color, Element, Length, Theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Menu,
    Sidebar,
    Search,
    History,
    Folder,
    File,
    ChevronRight,
    ChevronDown,
    Branch,
    Terminal,
    Bug,
    Outline,
    Users,
    Panels,
    Plus,
    Minus,
    Close,
    Split,
    Sparkle,
    Send,
    Eye,
    More,
    Pin,
    Expand,
    Chat,
    Paperclip,
}

macro_rules! svg_icon {
    ($body:expr) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">"#,
            $body,
            "</svg>"
        )
    };
}

impl Icon {
    fn source(self) -> &'static str {
        match self {
            Icon::Menu => svg_icon!(r#"<path d="M4 7h16M4 12h16M4 17h16"/>"#),
            Icon::Sidebar => svg_icon!(r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M9.5 4.5v15"/>"#),
            Icon::Search => svg_icon!(r#"<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>"#),
            Icon::History => svg_icon!(r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>"#),
            Icon::Folder => svg_icon!(r#"<path d="M3 7.5A2 2 0 0 1 5 5.5h4l2 2.2h8a2 2 0 0 1 2 2V17a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>"#),
            Icon::File => svg_icon!(r#"<path d="M7 3.5h7l4 4V19a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 6 19V5A1.5 1.5 0 0 1 7.5 3.5"/><path d="M14 3.5V8h4"/>"#),
            Icon::ChevronRight => svg_icon!(r#"<path d="M9.5 6l6 6-6 6"/>"#),
            Icon::ChevronDown => svg_icon!(r#"<path d="M6 9.5l6 6 6-6"/>"#),
            Icon::Branch => svg_icon!(r#"<circle cx="6" cy="5.5" r="2.2"/><circle cx="6" cy="18.5" r="2.2"/><circle cx="18" cy="8.5" r="2.2"/><path d="M6 7.7v8.6M18 10.7c0 3.6-3.4 5-8.6 6"/>"#),
            Icon::Terminal => svg_icon!(r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M7 9.5l3 2.5-3 2.5M12.5 15h4.5"/>"#),
            Icon::Bug => svg_icon!(r#"<path d="M8.5 10a3.5 3.5 0 0 1 7 0v4.5a3.5 3.5 0 0 1-7 0z"/><path d="M5 10.5l3.5 1.5M19 10.5L15.5 12M5 17.5l3.5-1.5M19 17.5L15.5 16M9.5 6.5L8.5 4M14.5 6.5L15.5 4"/>"#),
            Icon::Outline => svg_icon!(r#"<path d="M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01"/>"#),
            Icon::Users => svg_icon!(r#"<circle cx="9" cy="8" r="3"/><path d="M3.5 19a5.5 5.5 0 0 1 11 0"/><circle cx="17" cy="9.5" r="2.3"/><path d="M16 14.2a4.5 4.5 0 0 1 4.5 4.3"/>"#),
            Icon::Panels => svg_icon!(r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><rect x="14.5" y="4.5" width="6.5" height="15" rx="2.5" fill="white"/>"#),
            Icon::Plus => svg_icon!(r#"<path d="M12 5v14M5 12h14"/>"#),
            Icon::Minus => svg_icon!(r#"<path d="M5 12h14"/>"#),
            Icon::Close => svg_icon!(r#"<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>"#),
            Icon::Split => svg_icon!(r#"<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M12 4.5v15"/>"#),
            Icon::Sparkle => svg_icon!(r#"<path d="M12 3.5l1.9 5.6 5.6 1.9-5.6 1.9L12 18.5l-1.9-5.6-5.6-1.9 5.6-1.9z"/>"#),
            Icon::Send => svg_icon!(r#"<path d="M4 12l16-8-6 16-2.5-6.5z"/>"#),
            Icon::Eye => svg_icon!(r#"<path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/><circle cx="12" cy="12" r="2.8"/>"#),
            Icon::More => svg_icon!(r#"<circle cx="6" cy="12" r="1" fill="white"/><circle cx="12" cy="12" r="1" fill="white"/><circle cx="18" cy="12" r="1" fill="white"/>"#),
            Icon::Pin => svg_icon!(r#"<path d="M9 4h6l-1 6 3 3H7l3-3z"/><path d="M12 13v7"/>"#),
            Icon::Chat => svg_icon!(r#"<path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v7a2.5 2.5 0 0 1-2.5 2.5H11l-4.5 3.5V16A2.5 2.5 0 0 1 4 13.5z"/>"#),
            Icon::Paperclip => svg_icon!(r#"<path d="M20 11.5l-7.8 7.8a4.5 4.5 0 0 1-6.4-6.4l8-8a3 3 0 0 1 4.3 4.3l-8 8a1.5 1.5 0 0 1-2.1-2.1l7.4-7.4"/>"#),
            Icon::Expand => svg_icon!(r#"<path d="M14 4h6v6M10 20H4v-6M20 4l-7 7M4 20l7-7"/>"#),
        }
    }
}

/// An icon `size` pixels square, tinted `color`.
pub fn icon<'a, M: 'a>(icon: Icon, size: f32, color: Color) -> Element<'a, M> {
    svg(svg::Handle::from_memory(icon.source().as_bytes()))
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .style(move |_: &Theme, _| svg::Style { color: Some(color) })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_well_formed_svg() {
        for icon in [
            Icon::Menu, Icon::Sidebar, Icon::Search, Icon::History, Icon::Folder, Icon::File,
            Icon::ChevronRight, Icon::ChevronDown, Icon::Branch, Icon::Terminal, Icon::Bug,
            Icon::Outline, Icon::Users, Icon::Panels, Icon::Plus, Icon::Minus, Icon::Close, Icon::Split,
            Icon::Sparkle, Icon::Send, Icon::Eye, Icon::More, Icon::Pin, Icon::Expand,
            Icon::Chat, Icon::Paperclip,
        ] {
            let s = icon.source();
            assert!(s.starts_with("<svg") && s.ends_with("</svg>"), "{icon:?}");
            assert_eq!(s.matches('<').count(), s.matches('>').count(), "{icon:?}: unbalanced tags");
        }
    }
}
