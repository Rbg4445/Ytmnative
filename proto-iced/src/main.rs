//! YTM Native — iced prototype of the SimpMusic-style home screen.
//!
//! Purpose: see how close a native Rust UI (iced 0.13) gets to the WebView design,
//! and how much RAM it needs. Audio is not played; the player only simulates progress.
//!
//! Env vars:
//!   YTM_PROTO_QUERY  search used to load tracks (default: "türkçe pop hit")
//!   YTM_PROTO_SHOT   if set, saves a PNG screenshot to this path after loading, then exits

use std::collections::HashSet;
use std::f32::consts::PI;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use iced::alignment::{Horizontal, Vertical};
use iced::font::Weight;
use iced::gradient::Linear;
use iced::widget::{
    button, column, container, horizontal_space, image, opaque, row, scrollable, stack, svg, text,
    text_input, vertical_space, Space,
};
use iced::{
    window, Background, Border, Color, ContentFit, Element, Font, Gradient, Length, Padding,
    Radians, Shadow, Subscription, Task, Theme, Vector,
};

// ---------- Theme tokens (same values as the WebView version) ----------
const BG: Color = rgb(0x0b, 0x0b, 0x0d);
const CARD: Color = rgb(0x14, 0x14, 0x16);
const TEXT: Color = rgb(0xf2, 0xf2, 0xf4);
const MUTED: Color = rgb(0xa2, 0xa2, 0xa8);
const ACCENT: Color = rgb(0x8c, 0xcb, 0xf5);
const ON_ACCENT: Color = rgb(0x06, 0x2a, 0x40);
const LINE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.07 };
const LINE_STRONG: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.16 };
const HEART: Color = rgb(0xef, 0x44, 0x44);

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
}
fn white(a: f32) -> Color {
    Color { a, ..Color::WHITE }
}
fn black(a: f32) -> Color {
    Color { a, ..Color::BLACK }
}

const BASE_FONT: Font = Font::with_name("Segoe UI");
const SEMIBOLD: Font = Font { weight: Weight::Semibold, ..BASE_FONT };
const BOLD: Font = Font { weight: Weight::Bold, ..BASE_FONT };

// ---------- Icons (same paths as the HTML version) ----------
const I_HOME: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M12 3.2 4 9.6V20h5.5v-6h5v6H20V9.6l-8-6.4Z"/></svg>"#;
const I_MIX: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4.9 19.1a10 10 0 0 1 0-14.2M7.8 16.2a6 6 0 0 1 0-8.4M16.2 7.8a6 6 0 0 1 0 8.4M19.1 4.9a10 10 0 0 1 0 14.2"/><circle cx="12" cy="12" r="2" fill="white"/></svg>"#;
const I_EXPLORE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><circle cx="12" cy="12" r="9"/><polygon points="10,10 16,8 14,14 8,16" fill="white"/></svg>"#;
const I_LIBRARY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 6v14M9 6v14M14 7l5 13"/></svg>"#;
#[allow(dead_code)]
const I_SEARCH: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
const I_PLUS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M12 5v14M5 12h14"/></svg>"#;
const I_PANEL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><rect x="3" y="4" width="18" height="16" rx="3"/><path d="M15 4v16"/></svg>"#;
const I_SHUFFLE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 3h5v5M4 20 21 3M21 16v5h-5M15 15l6 6M4 4l5 5"/></svg>"#;
const I_PREV: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 6h2v12H6zm3.5 6 8.5 6V6z"/></svg>"#;
const I_NEXT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z"/></svg>"#;
const I_PLAY: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M8 5v14l11-7z"/></svg>"#;
const I_PAUSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M6 19h4V5H6zm8-14v14h4V5z"/></svg>"#;
const I_REPEAT: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 1l4 4-4 4M3 11V9a4 4 0 0 1 4-4h14M7 23l-4-4 4-4M21 13v2a4 4 0 0 1-4 4H3"/></svg>"#;
const I_HEART: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78Z"/></svg>"#;
const I_HEART_FULL: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M12 21.2 3.9 13.3a5.4 5.4 0 0 1 7.6-7.7l.5.5.5-.5a5.4 5.4 0 0 1 7.6 7.7L12 21.2Z"/></svg>"#;
const I_QUEUE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 6h12M4 12h12M4 18h8M18 14v6l4-3-4-3Z"/></svg>"#;
const I_LYRICS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 5h16v10H8l-4 4V5Z"/><path d="M8 9h8M8 12h5"/></svg>"#;
const I_VOLUME: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 7.97v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/></svg>"#;
const I_CHEVRON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const I_HEADPHONES: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2" stroke-linecap="round"><path d="M4 15v-3a8 8 0 0 1 16 0v3"/><rect x="3" y="14" width="5" height="7" rx="2"/><rect x="16" y="14" width="5" height="7" rx="2"/></svg>"#;
const I_CHECK: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"><path d="m5 12 5 5 9-10"/></svg>"#;
#[allow(dead_code)]
const I_PIN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M16 3l5 5-3 1-4 4 1 5-2 2-4-4-5 5-1-1 5-5-4-4 2-2 5 1 4-4 1-3Z"/></svg>"#;
const I_NOTE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="white"><path d="M9 18V5l11-2v13" stroke="white" stroke-width="2" fill="none"/><circle cx="6" cy="18" r="3"/><circle cx="17" cy="16" r="3"/></svg>"#;
const I_SETTINGS: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="white" stroke-width="2"><circle cx="12" cy="12" r="3"/><circle cx="12" cy="12" r="8" stroke-dasharray="3 2.2"/></svg>"#;

fn icon<'a>(src: &'static str, size: f32, color: Color) -> Element<'a, Message> {
    svg(svg::Handle::from_memory(src.as_bytes()))
        .width(size)
        .height(size)
        .style(move |_, _| svg::Style { color: Some(color) })
        .into()
}

// ---------- Data ----------
#[derive(Debug, Clone)]
struct Item {
    title: String,
    artist: String,
    duration: u32,
    thumb: image::Handle,  // 96 px, rounded 10
    card: image::Handle,   // 256 px, rounded 18
    circle: image::Handle, // 128 px, circle
    /// Original compressed cover (JPEG/WebP, ~50-150 KB). The large panel image is decoded
    /// from it only for the song that is playing, instead of keeping one per song in RAM.
    raw: Option<Arc<Vec<u8>>>,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Page {
    Home,
    Mix,
    Explore,
    Library,
}

#[derive(Debug, Clone)]
enum Message {
    Loaded(Arc<Vec<Item>>, String),
    Select(usize),
    TogglePlay,
    Next,
    Prev,
    Nav(Page),
    Chip(usize),
    LibChip(usize),
    Query(String),
    Tick,
    ToggleLike,
    ToggleShuffle,
    ToggleRepeat,
    TogglePanel,
    PanelTab(usize),
    IntroFrame(Instant),
    TakeShot,
    Shot(window::Screenshot),
}

// ---------- Startup intro ("Powered by iced", Mojang Studios style) ----------
/// Text fades in over this long.
const INTRO_FADE_IN: f32 = 0.6;
/// The intro stays at least this long (and until the home data has loaded).
const INTRO_MIN_HOLD: f32 = 2.0;
/// Then the whole intro screen fades away over this long.
const INTRO_FADE_OUT: f32 = 0.9;

const ICED_LOGO: &[u8] = include_bytes!("../assets/iced-logo.svg");

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

struct App {
    items: Arc<Vec<Item>>,
    source: String,
    loading: bool,
    current: Option<usize>,
    playing: bool,
    pos: f32,
    page: Page,
    chip: usize,
    lib_chip: usize,
    query: String,
    liked: HashSet<usize>,
    shuffle: bool,
    repeat: bool,
    panel_open: bool,
    panel_tab: usize,
    shot_path: Option<String>,
    /// Large right-panel artwork for the current song only.
    art: Option<image::Handle>,
    intro_start: Instant,
    /// Seconds since start, updated every frame while the intro runs.
    intro_t: f32,
    /// When the fade-out began (seconds since start).
    intro_fade_from: Option<f32>,
    intro_done: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let query = std::env::var("YTM_PROTO_QUERY").unwrap_or_else(|_| "türkçe pop hit".into());
        (
            Self {
                items: Arc::new(Vec::new()),
                source: String::new(),
                loading: true,
                current: None,
                playing: false,
                pos: 0.0,
                page: Page::Home,
                chip: 0,
                lib_chip: 0,
                query: String::new(),
                liked: HashSet::new(),
                shuffle: false,
                repeat: false,
                panel_open: true,
                panel_tab: 1,
                shot_path: std::env::var("YTM_PROTO_SHOT").ok(),
                art: None,
                intro_start: Instant::now(),
                intro_t: 0.0,
                intro_fade_from: None,
                intro_done: false,
            },
            {
                let load = Task::perform(load_items(query), |(items, src)| Message::Loaded(Arc::new(items), src));
                // YTM_PROTO_SHOT_DELAY=ms takes the screenshot at a fixed time (e.g. mid-intro).
                match std::env::var("YTM_PROTO_SHOT_DELAY").ok().and_then(|v| v.parse::<u64>().ok()) {
                    Some(ms) if std::env::var("YTM_PROTO_SHOT").is_ok() => Task::batch([
                        load,
                        Task::perform(tokio::time::sleep(Duration::from_millis(ms)), |_| Message::TakeShot),
                    ]),
                    _ => load,
                }
            },
        )
    }

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::Loaded(items, src) => {
                self.items = items;
                self.source = src;
                self.loading = false;
                if !self.items.is_empty() {
                    self.current = Some(4.min(self.items.len() - 1));
                    self.playing = true;
                    self.pos = 72.0;
                    self.refresh_art();
                }
            }
            Message::IntroFrame(now) => {
                self.intro_t = now.duration_since(self.intro_start).as_secs_f32();
                if self.intro_fade_from.is_none() && self.intro_t >= INTRO_MIN_HOLD && !self.loading {
                    self.intro_fade_from = Some(self.intro_t);
                }
                if let Some(from) = self.intro_fade_from {
                    if self.intro_t - from >= INTRO_FADE_OUT {
                        self.intro_done = true;
                        // Default screenshot: shortly after the intro has finished.
                        if self.shot_path.is_some() && std::env::var("YTM_PROTO_SHOT_DELAY").is_err() {
                            return Task::perform(tokio::time::sleep(Duration::from_millis(400)), |_| {
                                Message::TakeShot
                            });
                        }
                    }
                }
            }
            Message::Select(i) => {
                self.current = Some(i);
                self.playing = true;
                self.pos = 0.0;
                self.refresh_art();
            }
            Message::TogglePlay => self.playing = !self.playing,
            Message::Next | Message::Prev => {
                if let Some(c) = self.current {
                    let n = self.items.len().max(1);
                    self.current = Some(if matches!(msg, Message::Next) { (c + 1) % n } else { (c + n - 1) % n });
                    self.pos = 0.0;
                    self.refresh_art();
                }
            }
            Message::Nav(p) => self.page = p,
            Message::Chip(c) => self.chip = c,
            Message::LibChip(c) => self.lib_chip = c,
            Message::Query(q) => self.query = q,
            Message::Tick => {
                if let Some(c) = self.current {
                    self.pos += 0.25;
                    if self.pos >= self.items[c].duration as f32 {
                        return Task::done(Message::Next);
                    }
                }
            }
            Message::ToggleLike => {
                if let Some(c) = self.current {
                    if !self.liked.remove(&c) {
                        self.liked.insert(c);
                    }
                }
            }
            Message::ToggleShuffle => self.shuffle = !self.shuffle,
            Message::ToggleRepeat => self.repeat = !self.repeat,
            Message::TogglePanel => self.panel_open = !self.panel_open,
            Message::PanelTab(t) => self.panel_tab = t,
            Message::TakeShot => {
                return window::get_latest()
                    .and_then(window::screenshot)
                    .map(Message::Shot);
            }
            Message::Shot(shot) => {
                if let Some(path) = &self.shot_path {
                    let (w, h) = (shot.size.width, shot.size.height);
                    if let Some(img) = ::image::RgbaImage::from_raw(w, h, shot.bytes.to_vec()) {
                        let _ = img.save(path);
                    }
                }
                return iced::exit();
            }
        }
        Task::none()
    }

    fn refresh_art(&mut self) {
        self.art = self.current.map(|c| {
            let it = &self.items[c];
            let src = it
                .raw
                .as_ref()
                .and_then(|b| ::image::load_from_memory(b).ok())
                .map(|i| i.to_rgba8())
                .unwrap_or_else(|| placeholder_image(it.index));
            panel_art(&src)
        });
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = if self.playing && self.shot_path.is_none() && self.intro_done {
            iced::time::every(Duration::from_millis(250)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };
        let intro = if self.intro_done {
            Subscription::none()
        } else {
            window::frames().map(Message::IntroFrame)
        };
        Subscription::batch([tick, intro])
    }

    /// Full-screen "Powered by iced" intro that fades in, waits, then fades the app in.
    fn intro(&self) -> Element<'_, Message> {
        let t = self.intro_t;
        let text_alpha = smoothstep(t / INTRO_FADE_IN);
        let screen_alpha = match self.intro_fade_from {
            Some(from) => 1.0 - smoothstep((t - from) / INTRO_FADE_OUT),
            None => 1.0,
        };
        let a = text_alpha * screen_alpha;

        // Loading bar: creeps forward while data loads, fills up once it is ready.
        let progress = if self.loading { (t / 6.0).min(0.85) } else { 1.0 };
        let bar_w = 220.0;
        let bar = stack![
            container(Space::new(bar_w, 3)).style(move |_| rounded_bg(white(0.12 * a), 2.0)),
            container(Space::new(bar_w * progress, 3)).style(move |_| rounded_bg(Color { a, ..ACCENT }, 2.0)),
        ];

        // Official iced logo (github.com/iced-rs/iced, docs/logo.svg), embedded in the binary.
        let logo = svg(svg::Handle::from_memory(ICED_LOGO))
            .width(128)
            .height(144)
            .opacity(a);

        let content = column![
            logo,
            Space::with_height(18),
            text("Powered by").size(18).color(Color { a: 0.75 * a, ..MUTED }),
            text("iced UI").size(56).font(BOLD).color(Color { a, ..Color::WHITE }),
            Space::with_height(26),
            bar,
        ]
        .align_x(Horizontal::Center)
        .spacing(2);

        opaque(
            container(content)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_| container::Style {
                    background: Some(Color { a: screen_alpha, ..BG }.into()),
                    ..Default::default()
                }),
        )
    }

    // ================= VIEW =================
    fn view(&self) -> Element<'_, Message> {
        let left = stack![
            row![self.sidebar(), self.center()].spacing(10),
            container(self.capsule())
                .width(Length::Fill)
                .height(Length::Fill)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Bottom)
                .padding(Padding { top: 0.0, right: 16.0, bottom: 12.0, left: 16.0 }),
        ];

        let mut body = row![left].spacing(10);
        if self.panel_open {
            body = body.push(self.panel());
        }

        let app: Element<'_, Message> = container(body)
            .padding(10)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style { background: Some(BG.into()), ..Default::default() })
            .into();

        if self.intro_done {
            app
        } else {
            stack![app, self.intro()].into()
        }
    }

    // ---------- Sidebar ----------
    fn sidebar(&self) -> Element<'_, Message> {
        let nav_item = |ic: &'static str, label: &'static str, page: Page| {
            let active = self.page == page;
            let fg = if active { ACCENT } else { TEXT };
            button(
                row![icon(ic, 20.0, if active { ACCENT } else { MUTED }), text(label).size(14).font(if active { SEMIBOLD } else { BASE_FONT }).color(fg)]
                    .spacing(14)
                    .align_y(Vertical::Center),
            )
            .width(Length::Fill)
            .padding([10, 16])
            .on_press(Message::Nav(page))
            .style(move |_, status| button::Style {
                background: Some(match (active, status) {
                    (true, _) => white(0.04).into(),
                    (_, button::Status::Hovered) => white(0.05).into(),
                    _ => Color::TRANSPARENT.into(),
                }),
                text_color: fg,
                border: Border { radius: 999.0.into(), width: 1.0, color: if active { LINE_STRONG } else { Color::TRANSPARENT } },
                shadow: Shadow::default(),
            })
        };

        let chips = row![
            chip("Tümü", self.lib_chip == 0, Message::LibChip(0)),
            chip("Listeler", self.lib_chip == 1, Message::LibChip(1)),
            chip("Sanatçılar", self.lib_chip == 2, Message::LibChip(2)),
        ]
        .spacing(6);

        // Library: favorites + one entry per artist (like the SimpMusic list)
        let mut list = column![lib_row(
            container(icon(I_HEART_FULL, 22.0, HEART))
                .width(48)
                .height(48)
                .center_x(48)
                .center_y(48)
                .style(|_| rounded_bg(white(0.07), 10.0))
                .into(),
            "Beğendiğim müzikler".into(),
            format!("Oynatma listesi · {} şarkı", self.liked.len()),
        )]
        .spacing(2);
        let mut seen = HashSet::new();
        for it in self.items.iter() {
            let name = it.artist.split(',').next().unwrap_or("").trim().to_string();
            if self.lib_chip == 1 || !seen.insert(name.clone()) {
                continue;
            }
            list = list.push(lib_row(
                image(it.circle.clone()).width(48).height(48).into(),
                name,
                "Sanatçı".into(),
            ));
        }

        container(
            column![
                column![
                    nav_item(I_HOME, "Ana Sayfa", Page::Home),
                    nav_item(I_MIX, "Mix", Page::Mix),
                    nav_item(I_EXPLORE, "Keşfet", Page::Explore),
                    nav_item(I_LIBRARY, "Kitaplık", Page::Library),
                ]
                .spacing(4),
                container(Space::new(Length::Fill, 1)).style(|_| rounded_bg(white(0.08), 0.0)).padding([0, 4]),
                row![
                    icon(I_LIBRARY, 20.0, MUTED),
                    text("Kitaplığın").size(15).font(SEMIBOLD).color(TEXT),
                    horizontal_space(),
                    round_icon_button(I_PLUS, Message::Nav(Page::Library)),
                    round_icon_button(I_PANEL, Message::TogglePanel),
                ]
                .spacing(8)
                .align_y(Vertical::Center)
                .padding([0, 6]),
                chips,
                scrollable(column![list, Space::with_height(84)])
                    .height(Length::Fill)
                    .direction(thin_scroll(false)),
            ]
            .spacing(12),
        )
        .width(280)
        .height(Length::Fill)
        .padding(12)
        .style(|_| card_style())
        .into()
    }

    // ---------- Center ----------
    fn center(&self) -> Element<'_, Message> {
        let search = text_input("Şarkı, albüm, sanatçı ara", &self.query)
            .on_input(Message::Query)
            .padding([10, 16])
            .size(13.5)
            .width(Length::Fill)
            .style(|_, status| text_input::Style {
                background: white(if matches!(status, text_input::Status::Focused) { 0.09 } else { 0.06 }).into(),
                border: Border {
                    radius: 999.0.into(),
                    width: 1.0,
                    color: if matches!(status, text_input::Status::Focused) { white(0.3) } else { white(0.1) },
                },
                icon: MUTED,
                placeholder: MUTED,
                value: TEXT,
                selection: Color { a: 0.4, ..ACCENT },
            });

        let actions = container(
            row![
                round_icon_button(I_PANEL, Message::TogglePanel),
                round_icon_button(I_SETTINGS, Message::Nav(Page::Library)),
            ]
            .spacing(2),
        )
        .padding(3)
        .style(|_| container::Style {
            background: Some(white(0.03).into()),
            border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
            ..Default::default()
        });

        let top = row![
            container(icon(I_CHECK, 16.0, ON_ACCENT))
                .width(36)
                .height(36)
                .center_x(36)
                .center_y(36)
                .style(|_| rounded_bg(ACCENT, 999.0)),
            container(search).padding([0, 8]).width(Length::Fill).max_width(460),
            horizontal_space(),
            actions
        ]
        .spacing(8)
        .align_y(Vertical::Center)
        .padding(Padding { top: 14.0, right: 16.0, bottom: 8.0, left: 16.0 });

        let content: Element<'_, Message> = if self.loading {
            container(text("Öneriler yükleniyor…").size(14).color(MUTED)).padding(30).into()
        } else {
            self.home_content()
        };

        container(column![top, scrollable(content).height(Length::Fill).direction(thin_scroll(false))])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(|_| container::Style {
                // soft accent glow at the top, like the WebView version
                background: Some(Background::Gradient(Gradient::Linear(
                    Linear::new(Radians(PI))
                        .add_stop(0.0, Color { a: 1.0, ..mix(CARD, ACCENT, 0.12) })
                        .add_stop(0.35, CARD)
                        .add_stop(1.0, CARD),
                ))),
                border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
                ..Default::default()
            })
            .into()
    }

    fn home_content(&self) -> Element<'_, Message> {
        let items = &self.items;
        let chips = row![
            chip("Tümü", self.chip == 0, Message::Chip(0)),
            chip("Ruh hali", self.chip == 1, Message::Chip(1)),
            chip("Listeler", self.chip == 2, Message::Chip(2)),
            chip("Sanatçılar", self.chip == 3, Message::Chip(3)),
        ]
        .spacing(8);

        // Quick picks: columns of 4 compact rows, scrolling sideways
        let mut qp_cols = row![].spacing(18);
        for chunk_start in (0..items.len().min(16)).step_by(4) {
            let mut col = column![].spacing(4).width(300);
            for i in chunk_start..(chunk_start + 4).min(items.len()) {
                col = col.push(self.track_row(i));
            }
            qp_cols = qp_cols.push(col);
        }
        let quick_picks = column![
            row![
                column![
                    text("BİR RADYO İLE BAŞLA").size(11).font(SEMIBOLD).color(MUTED),
                    text("Hızlı seçimler").size(21).font(SEMIBOLD).color(TEXT),
                ]
                .spacing(2),
                horizontal_space(),
                button(text("Tümünü çal").size(12.5).color(MUTED))
                    .style(|_, _| button::Style { background: None, text_color: MUTED, ..Default::default() })
                    .on_press(Message::Select(0)),
            ]
            .align_y(Vertical::Bottom),
            scrollable(container(qp_cols).padding(Padding { top: 0.0, right: 0.0, bottom: 10.0, left: 0.0 }))
                .direction(thin_scroll(true)),
        ]
        .spacing(12);

        // Big square cards with the title on the art
        let mut cards = row![].spacing(14);
        for (i, it) in items.iter().enumerate().rev().take(12) {
            cards = cards.push(self.big_card(i, it));
        }

        // Artists
        let mut artists = row![].spacing(12);
        let mut seen = HashSet::new();
        for it in items.iter() {
            let name = it.artist.split(',').next().unwrap_or("").trim().to_string();
            if !seen.insert(name.clone()) {
                continue;
            }
            artists = artists.push(
                column![
                    image(it.circle.clone()).width(128).height(128),
                    text(ellipsize(&name, 16)).size(14).font(SEMIBOLD).color(TEXT),
                    text("Sanatçı").size(12).color(MUTED),
                ]
                .spacing(6)
                .width(150)
                .align_x(Horizontal::Center),
            );
        }

        column![
            chips,
            quick_picks,
            section("YouTube geçmişine göre öneriler", scrollable(cards).direction(thin_scroll(true)).into()),
            section("Favori sanatçıların", scrollable(artists).direction(thin_scroll(true)).into()),
            Space::with_height(100),
        ]
        .spacing(30)
        .padding(Padding { top: 4.0, right: 24.0, bottom: 0.0, left: 24.0 })
        .into()
    }

    fn track_row(&self, i: usize) -> Element<'_, Message> {
        let it = &self.items[i];
        let active = self.current == Some(i);
        button(
            row![
                image(it.thumb.clone()).width(48).height(48),
                column![
                    text(ellipsize(&it.title, 30)).size(14).font(SEMIBOLD).color(if active { ACCENT } else { TEXT }),
                    text(ellipsize(&it.artist, 36)).size(12).color(MUTED),
                ]
                .spacing(2),
            ]
            .spacing(12)
            .align_y(Vertical::Center),
        )
        .width(Length::Fill)
        .padding(6)
        .on_press(Message::Select(i))
        .style(move |_, status| button::Style {
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
            border: Border { radius: 12.0.into(), ..Default::default() },
            shadow: Shadow::default(),
        })
        .into()
    }

    fn big_card<'a>(&'a self, i: usize, it: &'a Item) -> Element<'a, Message> {
        let active = self.current == Some(i);
        let art = stack![
            image(it.card.clone()).width(220).height(220),
            container(Space::new(220, 220)).style(|_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(
                    Linear::new(Radians(PI))
                        .add_stop(0.45, Color::TRANSPARENT)
                        .add_stop(1.0, black(0.8)),
                ))),
                border: Border { radius: 18.0.into(), ..Default::default() },
                ..Default::default()
            }),
            container(text(ellipsize(&it.title, 22)).size(17).font(BOLD).color(Color::WHITE))
                .width(220)
                .height(220)
                .align_bottom(220)
                .padding(14),
        ];
        column![
            button(art)
                .padding(0)
                .on_press(Message::Select(i))
                .style(move |_, _| button::Style {
                    background: None,
                    text_color: TEXT,
                    border: Border {
                        radius: 18.0.into(),
                        width: if active { 2.0 } else { 0.0 },
                        color: ACCENT,
                    },
                    shadow: Shadow::default(),
                }),
            text(ellipsize(&it.artist, 28)).size(12.5).color(MUTED),
        ]
        .spacing(8)
        .width(220)
        .into()
    }

    // ---------- Floating player capsule ----------
    fn capsule(&self) -> Element<'_, Message> {
        let cur = self.current.map(|c| &self.items[c]);
        let (title, artist, dur) = cur
            .map(|it| (it.title.as_str(), it.artist.as_str(), it.duration as f32))
            .unwrap_or(("Şarkı seçilmedi", "", 1.0));
        let pct = (self.pos / dur.max(1.0)).clamp(0.0, 1.0);
        let liked = self.current.map(|c| self.liked.contains(&c)).unwrap_or(false);

        let progress = row![
            container(Space::new(Length::Fill, 3)).width(Length::FillPortion((pct * 1000.0) as u16 + 1)).style(|_| rounded_bg(Color::WHITE, 2.0)),
            container(Space::new(Length::Fill, 3)).width(Length::FillPortion(((1.0 - pct) * 1000.0) as u16 + 1)).style(|_| rounded_bg(white(0.16), 2.0)),
        ];

        let info = row![
            match cur {
                Some(it) => Element::from(image(it.thumb.clone()).width(40).height(40)),
                None => Space::new(40, 40).into(),
            },
            column![
                row![
                    text(ellipsize(title, 34)).size(13.5).font(SEMIBOLD).color(TEXT),
                    horizontal_space(),
                    text(format!("{} / {}", fmt_time(self.pos), fmt_time(dur))).size(10.5).color(MUTED),
                ],
                text(ellipsize(artist, 40)).size(12).color(MUTED),
                progress,
            ]
            .spacing(3)
            .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Vertical::Center)
        .width(Length::Fill);

        let content = row![
            pbtn(I_SHUFFLE, 20.0, self.shuffle, Message::ToggleShuffle),
            pbtn(I_PREV, 20.0, false, Message::Prev),
            pbtn(if self.playing { I_PAUSE } else { I_PLAY }, 28.0, false, Message::TogglePlay),
            pbtn(I_NEXT, 20.0, false, Message::Next),
            pbtn(I_REPEAT, 20.0, self.repeat, Message::ToggleRepeat),
            divider(),
            info,
            divider(),
            button(icon(if liked { I_HEART_FULL } else { I_HEART }, 20.0, if liked { HEART } else { TEXT }))
                .padding(7)
                .on_press(Message::ToggleLike)
                .style(|_, s| round_btn_style(s)),
            pbtn(I_QUEUE, 20.0, false, Message::PanelTab(2)),
            pbtn(I_LYRICS, 20.0, false, Message::PanelTab(0)),
            pbtn(I_PANEL, 20.0, self.panel_open, Message::TogglePanel),
            pbtn(I_VOLUME, 20.0, false, Message::TogglePlay),
            container(Space::new(Length::Fill, 3)).width(70).style(|_| rounded_bg(white(0.6), 2.0)),
        ]
        .spacing(4)
        .align_y(Vertical::Center);

        container(content)
            .width(Length::Fill)
            .max_width(940)
            .height(64)
            .padding(Padding { top: 0.0, right: 18.0, bottom: 0.0, left: 12.0 })
            .center_y(64)
            .style(|_| container::Style {
                background: Some(Color { a: 0.94, ..rgb(0x1c, 0x1c, 0x20) }.into()),
                border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
                shadow: Shadow { color: black(0.55), offset: Vector::new(0.0, 16.0), blur_radius: 40.0 },
                ..Default::default()
            })
            .into()
    }

    // ---------- Right now-playing panel ----------
    fn panel(&self) -> Element<'_, Message> {
        let Some(c) = self.current else {
            return container(text("Henüz bir parça çalmıyor").color(MUTED))
                .width(380)
                .height(Length::Fill)
                .center_x(380)
                .center_y(Length::Fill)
                .style(|_| card_style())
                .into();
        };
        let it = &self.items[c];
        let liked = self.liked.contains(&c);

        let tab = |ic: &'static str, idx: usize| {
            let active = self.panel_tab == idx;
            button(container(icon(ic, 19.0, Color::WHITE)).center_x(58).center_y(36))
                .width(58)
                .height(36)
                .padding(0)
                .on_press(Message::PanelTab(idx))
                .style(move |_, _| button::Style {
                    background: Some(if active { white(0.18) } else { Color::TRANSPARENT }.into()),
                    text_color: Color::WHITE,
                    border: Border { radius: 999.0.into(), ..Default::default() },
                    shadow: Shadow::default(),
                })
        };

        let info = column![
            vertical_space(),
            row![
                column![
                    text(ellipsize(&it.title, 26)).size(21).font(BOLD).color(Color::WHITE),
                    text(ellipsize(&it.artist, 34)).size(14).color(white(0.7)),
                ]
                .spacing(4)
                .width(Length::Fill),
                button(icon(I_MIX, 20.0, Color::WHITE)).padding(8).on_press(Message::Select(c)).style(|_, s| round_btn_style(s)),
                button(icon(if liked { I_HEART_FULL } else { I_HEART }, 20.0, if liked { HEART } else { Color::WHITE }))
                    .padding(8)
                    .on_press(Message::ToggleLike)
                    .style(|_, s| round_btn_style(s)),
            ]
            .align_y(Vertical::Center),
            row![icon(I_NOTE, 15.0, white(0.9)), text("Söz satırı burada görünür").size(15).color(white(0.9))].spacing(8).align_y(Vertical::Center),
            container(
                container(row![tab(I_LYRICS, 0), tab(I_HEADPHONES, 1), tab(I_QUEUE, 2)])
                    .padding(3)
                    .style(|_| rounded_bg(white(0.1), 999.0)),
            )
            .width(Length::Fill)
            .center_x(Length::Fill),
        ]
        .spacing(14);

        let collapse = button(icon(I_CHEVRON, 20.0, Color::WHITE))
            .padding(10)
            .on_press(Message::TogglePanel)
            .style(|_, s| button::Style {
                background: Some(black(if matches!(s, button::Status::Hovered) { 0.55 } else { 0.35 }).into()),
                text_color: Color::WHITE,
                border: Border { radius: 999.0.into(), ..Default::default() },
                shadow: Shadow::default(),
            });

        container(stack![
            match &self.art {
                Some(h) => Element::from(image(h.clone()).width(Length::Fill).height(Length::Fill).content_fit(ContentFit::Fill)),
                None => Space::new(Length::Fill, Length::Fill).into(),
            },
            container(Space::new(Length::Fill, Length::Fill)).style(|_| container::Style {
                background: Some(Background::Gradient(Gradient::Linear(
                    Linear::new(Radians(PI))
                        .add_stop(0.0, black(0.25))
                        .add_stop(0.3, Color::TRANSPARENT)
                        .add_stop(0.62, black(0.55))
                        .add_stop(1.0, black(0.9)),
                ))),
                border: Border { radius: 18.0.into(), ..Default::default() },
                ..Default::default()
            }),
            container(column![collapse, info]).padding(16).width(Length::Fill).height(Length::Fill),
        ])
        .width(380)
        .height(Length::Fill)
        .style(|_| container::Style {
            background: Some(CARD.into()),
            border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
            ..Default::default()
        })
        .into()
    }
}

// ---------- Small widgets & styles ----------
fn card_style() -> container::Style {
    container::Style {
        background: Some(CARD.into()),
        border: Border { radius: 18.0.into(), width: 1.0, color: LINE },
        ..Default::default()
    }
}

fn rounded_bg(c: Color, r: f32) -> container::Style {
    container::Style {
        background: Some(c.into()),
        border: Border { radius: r.into(), ..Default::default() },
        ..Default::default()
    }
}

fn chip(label: &str, active: bool, msg: Message) -> Element<'_, Message> {
    button(text(label).size(12.5).font(if active { SEMIBOLD } else { BASE_FONT }))
        .padding([5, 16])
        .on_press(msg)
        .style(move |_, status| button::Style {
            background: Some(
                if active {
                    ACCENT
                } else if matches!(status, button::Status::Hovered) {
                    white(0.06)
                } else {
                    Color::TRANSPARENT
                }
                .into(),
            ),
            text_color: if active { ON_ACCENT } else { TEXT },
            border: Border { radius: 999.0.into(), width: 1.0, color: if active { ACCENT } else { LINE_STRONG } },
            shadow: Shadow::default(),
        })
        .into()
}

fn lib_row<'a>(thumb: Element<'a, Message>, title: String, sub: String) -> Element<'a, Message> {
    button(
        row![
            thumb,
            column![
                text(ellipsize(&title, 24)).size(14).font(SEMIBOLD).color(TEXT),
                text(ellipsize(&sub, 30)).size(12).color(MUTED),
            ]
            .spacing(2),
        ]
        .spacing(12)
        .align_y(Vertical::Center),
    )
    .width(Length::Fill)
    .padding([6, 8])
    .on_press(Message::Nav(Page::Library))
    .style(|_, status| button::Style {
        background: Some(if matches!(status, button::Status::Hovered) { white(0.05) } else { Color::TRANSPARENT }.into()),
        text_color: TEXT,
        border: Border { radius: 12.0.into(), ..Default::default() },
        shadow: Shadow::default(),
    })
    .into()
}

fn round_icon_button(ic: &'static str, msg: Message) -> Element<'static, Message> {
    button(icon(ic, 17.0, TEXT))
        .padding(8)
        .on_press(msg)
        .style(|_, status| button::Style {
            background: Some(if matches!(status, button::Status::Hovered) { white(0.08) } else { Color::TRANSPARENT }.into()),
            text_color: TEXT,
            border: Border { radius: 999.0.into(), width: 1.0, color: LINE_STRONG },
            shadow: Shadow::default(),
        })
        .into()
}

fn round_btn_style(status: button::Status) -> button::Style {
    button::Style {
        background: Some(if matches!(status, button::Status::Hovered) { white(0.12) } else { Color::TRANSPARENT }.into()),
        text_color: TEXT,
        border: Border { radius: 999.0.into(), ..Default::default() },
        shadow: Shadow::default(),
    }
}

fn pbtn(ic: &'static str, size: f32, on: bool, msg: Message) -> Element<'static, Message> {
    button(icon(ic, size, if on { ACCENT } else { TEXT }))
        .padding(7)
        .on_press(msg)
        .style(|_, s| round_btn_style(s))
        .into()
}

fn divider() -> Element<'static, Message> {
    container(container(Space::new(1, 30)).style(|_| rounded_bg(LINE_STRONG, 0.0)))
        .padding([0, 6])
        .into()
}

fn section<'a>(title: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![text(title).size(21).font(SEMIBOLD).color(TEXT), body].spacing(14).into()
}

fn thin_scroll(horizontal: bool) -> scrollable::Direction {
    let bar = scrollable::Scrollbar::new().width(4).scroller_width(4).margin(2);
    if horizontal {
        scrollable::Direction::Horizontal(bar)
    } else {
        scrollable::Direction::Vertical(bar)
    }
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: 1.0 }
}

fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>().trim_end())
    }
}

fn fmt_time(secs: f32) -> String {
    let s = secs.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

// ---------- Loading: real tracks + thumbnails, placeholders when offline ----------
struct RawTrack {
    title: String,
    artist: String,
    duration: u32,
    url: Option<String>,
}

async fn fetch_tracks(query: &str) -> Result<Vec<RawTrack>, String> {
    let rp = rustypipe::client::RustyPipe::new();
    let res = tokio::time::timeout(Duration::from_secs(15), rp.query().music_search_tracks(query))
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| e.to_string())?;
    Ok(res
        .items
        .items
        .into_iter()
        .take(20)
        .map(|t| RawTrack {
            title: t.name,
            artist: t.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", "),
            duration: t.duration.unwrap_or(200),
            url: t.cover.iter().max_by_key(|c| c.width).map(|c| upsize_cover(&c.url)),
        })
        .collect())
}

async fn load_items(query: String) -> (Vec<Item>, String) {
    let tracks = match fetch_tracks(&query).await {
        Ok(t) if !t.is_empty() => t,
        Ok(_) | Err(_) => return (placeholder_items(), "çevrimdışı örnek veriler".into()),
    };
    let http = reqwest::Client::builder().timeout(Duration::from_secs(12)).build().unwrap();
    let jobs = tracks.into_iter().enumerate().map(|(i, t)| {
        let http = http.clone();
        async move {
            let bytes = match &t.url {
                Some(u) => match http.get(u).send().await {
                    Ok(r) => r.bytes().await.ok(),
                    Err(_) => None,
                },
                None => None,
            };
            let raw = bytes.map(|b| Arc::new(b.to_vec()));
            let raw2 = raw.clone();
            let imgs = tokio::task::spawn_blocking(move || {
                raw2.and_then(|b| ::image::load_from_memory(&b).ok())
                    .map(|i| i.to_rgba8())
                    .unwrap_or_else(|| placeholder_image(i))
            })
            .await
            .unwrap_or_else(|_| placeholder_image(i));
            make_item(t.title, t.artist, t.duration, imgs, raw, i)
        }
    });
    let items: Vec<Item> = futures::stream::iter(jobs).buffered(8).collect().await;
    (items, format!("YouTube Music: \"{query}\""))
}

fn make_item(
    title: String,
    artist: String,
    duration: u32,
    src: ::image::RgbaImage,
    raw: Option<Arc<Vec<u8>>>,
    index: usize,
) -> Item {
    use ::image::imageops::{self, FilterType};
    // center square crop
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let sq = imageops::crop_imm(&src, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let resize = |s: u32| imageops::resize(&sq, s, s, FilterType::Triangle);

    let thumb = round_corners(resize(96), 10.0 * 2.0);
    let card = round_corners(resize(256), 18.0 * 256.0 / 220.0);
    let circle = round_corners(resize(128), 64.0);

    let handle = |img: ::image::RgbaImage| {
        let (w, h) = img.dimensions();
        image::Handle::from_rgba(w, h, img.into_raw())
    };
    Item { title, artist, duration, thumb: handle(thumb), card: handle(card), circle: handle(circle), raw, index }
}

/// Tall crop for the right panel (380 x ~760 logical px) with rounded corners baked in.
fn panel_art(src: &::image::RgbaImage) -> image::Handle {
    use ::image::imageops::{self, FilterType};
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let strip_w = side / 2;
    let tall = imageops::crop_imm(src, (w - strip_w) / 2, (h - side) / 2, strip_w, side).to_image();
    let art = round_corners(imageops::resize(&tall, 420, 840, FilterType::CatmullRom), 18.0 * 420.0 / 380.0);
    let (aw, ah) = art.dimensions();
    image::Handle::from_rgba(aw, ah, art.into_raw())
}

/// YouTube Music cover URLs carry their size ("...=w120-h120-l90-rj"); ask for 1000 px.
fn upsize_cover(url: &str) -> String {
    if url.contains("googleusercontent.com") || url.contains("ggpht.com") {
        if let Some(pos) = url.rfind('=') {
            return format!("{}=w1000-h1000-l90-rj", &url[..pos]);
        }
    }
    url.to_string()
}

/// Bake anti-aliased rounded corners into the pixels (iced 0.13 images are not clipped by
/// their container's border radius).
fn round_corners(mut img: ::image::RgbaImage, radius: f32) -> ::image::RgbaImage {
    let (w, h) = img.dimensions();
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let cx = if px < r { r } else if px > w as f32 - r { w as f32 - r } else { px };
            let cy = if py < r { r } else if py > h as f32 - r { h as f32 - r } else { py };
            let d = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
            let cover = (r - d + 0.5).clamp(0.0, 1.0);
            if cover < 1.0 {
                let p = img.get_pixel_mut(x, y);
                p[3] = (p[3] as f32 * cover) as u8;
            }
        }
    }
    img
}

fn placeholder_image(i: usize) -> ::image::RgbaImage {
    const COLS: [[u8; 3]; 8] = [
        [0xe1, 0x1d, 0x48], [0x7c, 0x3a, 0xed], [0x08, 0x91, 0xb2], [0xca, 0x8a, 0x04],
        [0x16, 0xa3, 0x4a], [0xdb, 0x27, 0x77], [0x25, 0x63, 0xeb], [0xea, 0x58, 0x0c],
    ];
    let c = COLS[i % 8];
    ::image::RgbaImage::from_fn(256, 256, |x, y| {
        let t = (x + y) as f32 / 512.0;
        let f = |v: u8| (v as f32 * (1.0 - t) + 17.0 * t) as u8;
        ::image::Rgba([f(c[0]), f(c[1]), f(c[2]), 255])
    })
}

fn placeholder_items() -> Vec<Item> {
    let names = [
        ("Midnight City", "M83"), ("Blinding Lights", "The Weeknd"), ("Yalnızlık Senfonisi", "Sezen Aksu"),
        ("Ocean Eyes", "Billie Eilish"), ("Bir Derdim Var", "mor ve ötesi"), ("Levitating", "Dua Lipa"),
        ("Sen Ağlama", "Duman"), ("Starboy", "The Weeknd"), ("Kuzu Kuzu", "Tarkan"),
        ("Harbiye Açıkhava Canlı", "Teoman"), ("Dreams", "Fleetwood Mac"), ("Bad Guy", "Billie Eilish"),
        ("Get Lucky", "Daft Punk"), ("Şımarık", "Tarkan"), ("Ölürüm Sana", "Tarkan"), ("Gece", "Duman"),
    ];
    names
        .iter()
        .enumerate()
        .map(|(i, (t, a))| make_item(t.to_string(), a.to_string(), 180 + i as u32 * 7, placeholder_image(i), None, i))
        .collect()
}

fn main() -> iced::Result {
    iced::application("YTM Native — iced prototipi", App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| Theme::Dark)
        .window_size((1240.0, 780.0))
        .default_font(BASE_FONT)
        .antialiasing(true)
        .run_with(App::new)
}
