//! Colors, fonts, icons and small style helpers (same tokens as the 7.x WebView design).

use iced::font::Weight;
use iced::widget::{button, container, scrollable, svg};
use iced::{Border, Color, Element, Font, Shadow};

pub const BG: Color = rgb(0x0b, 0x0b, 0x0d);
pub const CARD: Color = rgb(0x14, 0x14, 0x16);
pub const TEXT: Color = rgb(0xf2, 0xf2, 0xf4);
pub const MUTED: Color = rgb(0xa2, 0xa2, 0xa8);
pub const ACCENT: Color = rgb(0x8c, 0xcb, 0xf5);
pub const ON_ACCENT: Color = rgb(0x06, 0x2a, 0x40);
pub const LINE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.07 };
pub const LINE_STRONG: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.16 };
pub const HEART: Color = rgb(0xef, 0x44, 0x44);
pub const ERROR_BG: Color = rgb(0x3a, 0x12, 0x18);

pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}
pub fn white(a: f32) -> Color {
    Color { a, ..Color::WHITE }
}
pub fn black(a: f32) -> Color {
    Color { a, ..Color::BLACK }
}
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1.0 }
}

pub const BASE_FONT: Font = Font::with_name("Segoe UI");
pub const SEMIBOLD: Font = Font { weight: Weight::Semibold, ..BASE_FONT };
pub const BOLD: Font = Font { weight: Weight::Bold, ..BASE_FONT };

pub const ICED_LOGO: &[u8] = include_bytes!("../assets/iced-logo.svg");
pub const APP_ICON_PNG: &[u8] = include_bytes!("../../icons/icon.png");

// ---------- Icons ----------
pub const I_HOME: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M12 3.2 4 9.6V20h5.5v-6h5v6H20V9.6l-8-6.4Z"/></svg>"#;
pub const I_MIX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4.9 19.1a10 10 0 0 1 0-14.2M7.8 16.2a6 6 0 0 1 0-8.4M16.2 7.8a6 6 0 0 1 0 8.4M19.1 4.9a10 10 0 0 1 0 14.2"/><circle cx="12" cy="12" r="2" fill="white"/></svg>"#;
pub const I_EXPLORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><circle cx="12" cy="12" r="9"/><polygon points="10,10 16,8 14,14 8,16" fill="white"/></svg>"#;
pub const I_LIBRARY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 6v14M9 6v14M14 7l5 13"/></svg>"#;
pub const I_SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
pub const I_PANEL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M15 4v16"/></svg>"#;
pub const I_SHUFFLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5M4 20 21 3M21 16v5h-5M15 15l6 6M4 4l5 5"/></svg>"#;
pub const I_PREV: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z"/></svg>"#;
pub const I_NEXT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>"#;
pub const I_PLAY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M8 5v14l11-7z"/></svg>"#;
pub const I_PAUSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 19h4V5H6zm8-14v14h4V5z"/></svg>"#;
pub const I_REPEAT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 1l4 4-4 4M3 11V9a4 4 0 0 1 4-4h14M7 23l-4-4 4-4M21 13v2a4 4 0 0 1-4 4H3"/></svg>"#;
pub const I_HEART: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78Z"/></svg>"#;
pub const I_HEART_FULL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M12 21.2 3.9 13.3a5.4 5.4 0 0 1 7.6-7.7l.5.5.5-.5a5.4 5.4 0 0 1 7.6 7.7L12 21.2Z"/></svg>"#;
pub const I_QUEUE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 6h12M4 12h12M4 18h8M18 14v6l4-3-4-3Z"/></svg>"#;
pub const I_LYRICS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5h16v10H8l-4 4V5Z"/><path d="M8 9h8M8 12h5"/></svg>"#;
pub const I_VOLUME: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/></svg>"#;
pub const I_CHEVRON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
pub const I_BACK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m15 18-6-6 6-6"/></svg>"#;
pub const I_HEADPHONES: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="14" width="5" height="7" rx="2"/><rect x="16" y="14" width="5" height="7" rx="2"/></svg>"#;
pub const I_CHECK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 5 5 9-10"/></svg>"#;
pub const I_USER: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><circle cx="12" cy="8" r="4"/><path d="M4 21a8 8 0 0 1 16 0"/></svg>"#;
pub const I_NOTE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M9 18V5l11-2v13" stroke="white" stroke-width="2" fill="none"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/></svg>"#;
pub const I_REFRESH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/></svg>"#;
pub const I_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9c.26.6.85 1 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"/></svg>"#;
pub const I_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>"#;
pub const I_MINI: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><rect x="11" y="11" width="8" height="6" rx="1" fill="white"/></svg>"#;
pub const I_MOON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8Z"/></svg>"#;
pub const I_TRASH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6"/></svg>"#;
pub const I_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.2" stroke-linecap="round"><path d="M6 6l12 12M18 6 6 18"/></svg>"#;
pub const I_DOWNLOAD: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v12m0 0-4-4m4 4 4-4M4 19h16"/></svg>"#;

// Window buttons (thin, Windows 11 / Discord style)
pub const I_WIN_MIN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M1 6h10" stroke="white" stroke-width="1"/></svg>"#;
pub const I_WIN_MAX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none"><rect x="1.5" y="1.5" width="9" height="9" rx="1" stroke="white" stroke-width="1"/></svg>"#;
pub const I_WIN_RESTORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="white" stroke-width="1"><rect x="1.5" y="3.5" width="7" height="7" rx="1"/><path d="M3.5 3.5V2.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1"/></svg>"#;
pub const I_WIN_CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12"><path d="M1.5 1.5l9 9M10.5 1.5l-9 9" stroke="white" stroke-width="1"/></svg>"#;

pub fn icon<'a, M: 'a>(src: &'static str, size: f32, color: Color) -> Element<'a, M> {
    svg(svg::Handle::from_memory(src.as_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

// ---------- Style helpers ----------
pub fn card_style() -> container::Style {
    container::Style {
        background: Some(CARD.into()),
        border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
        ..Default::default()
    }
}

pub fn rounded_bg(c: Color, r: f32) -> container::Style {
    container::Style {
        background: Some(c.into()),
        border: Border { radius: r.into(), ..Default::default() },
        ..Default::default()
    }
}

pub fn round_btn_style(status: button::Status) -> button::Style {
    button::Style {
        background: Some(if matches!(status, button::Status::Hovered) { white(0.12) } else { Color::TRANSPARENT }.into()),
        text_color: TEXT,
        border: Border { radius: 999.0.into(), ..Default::default() },
        shadow: Shadow::default(),
    }
}

pub fn row_btn_style(active: bool, status: button::Status, radius: f32) -> button::Style {
    button::Style {
        background: Some(
            if active {
                white(0.08)
            } else if matches!(status, button::Status::Hovered) {
                white(0.05)
            } else {
                Color::TRANSPARENT
            }
            .into(),
        ),
        text_color: TEXT,
        border: Border { radius: radius.into(), ..Default::default() },
        shadow: Shadow::default(),
    }
}

pub fn thin_scroll(horizontal: bool) -> scrollable::Direction {
    let bar = scrollable::Scrollbar::new().width(4).scroller_width(4).margin(2);
    if horizontal {
        scrollable::Direction::Horizontal(bar)
    } else {
        scrollable::Direction::Vertical(bar)
    }
}

pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
    }
}

pub fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Turkish short subscriber counts, like YouTube TR: 11,9 Mn / 9,8 B.
pub fn fmt_count(n: Option<u64>) -> String {
    let Some(n) = n.filter(|n| *n > 0) else { return "Sanatçı".into() };
    let short = |v: f64| format!("{v:.1}").trim_end_matches(".0").replace('.', ",");
    if n >= 1_000_000_000 {
        format!("{} Mr abone", short(n as f64 / 1e9))
    } else if n >= 1_000_000 {
        format!("{} Mn abone", short(n as f64 / 1e6))
    } else if n >= 1_000 {
        format!("{} B abone", short(n as f64 / 1e3))
    } else {
        format!("{n} abone")
    }
}
