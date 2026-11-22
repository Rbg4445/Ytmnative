//! Cover images: download, resize to the size they are shown at, bake rounded corners.
//! Keeping only display-sized RGBA (not full JPEGs decoded) is what keeps RAM low.

use std::time::Duration;

use iced::widget::image;
use image_crate::imageops::{self, FilterType};
use image_crate::RgbaImage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// 96 px square, small rounded corners (lists, quick picks, capsule)
    Thumb,
    /// 256 px square, 18 px corners (big cards)
    Card,
    /// 128 px circle (artists)
    Circle,
    /// 420x840 tall crop for the right now-playing panel
    Art,
}

/// YouTube Music cover URLs carry their size ("...=w120-h120-l90-rj"); ask for a bigger one.
pub fn sized_url(url: &str, kind: Kind) -> String {
    let px = match kind {
        Kind::Thumb => 226,
        Kind::Card | Kind::Circle => 544,
        Kind::Art => 1000,
    };
    if url.contains("googleusercontent.com") || url.contains("ggpht.com") {
        if let Some(pos) = url.rfind('=') {
            return format!("{}=w{px}-h{px}-l90-rj", &url[..pos]);
        }
    }
    url.to_string()
}

pub async fn load(http: reqwest::Client, url: String, kind: Kind) -> Option<image::Handle> {
    let bytes = http
        .get(sized_url(&url, kind))
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;
    tokio::task::spawn_blocking(move || process(&bytes, kind)).await.ok().flatten()
}

pub fn process(bytes: &[u8], kind: Kind) -> Option<image::Handle> {
    let src = image_crate::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = src.dimensions();
    let side = w.min(h);
    let sq = imageops::crop_imm(&src, (w - side) / 2, (h - side) / 2, side, side).to_image();
    let img = match kind {
        Kind::Thumb => round_corners(imageops::resize(&sq, 96, 96, FilterType::Triangle), 20.0),
        Kind::Card => round_corners(imageops::resize(&sq, 256, 256, FilterType::Triangle), 18.0 * 256.0 / 220.0),
        Kind::Circle => round_corners(imageops::resize(&sq, 128, 128, FilterType::Triangle), 64.0),
        Kind::Art => {
            let strip = side / 2;
            let tall = imageops::crop_imm(&sq, (side - strip) / 2, 0, strip, side).to_image();
            round_corners(imageops::resize(&tall, 420, 840, FilterType::CatmullRom), 18.0 * 420.0 / 380.0)
        }
    };
    let (iw, ih) = img.dimensions();
    Some(image::Handle::from_rgba(iw, ih, img.into_raw()))
}

/// Anti-aliased rounded corners baked into the pixels (iced 0.13 doesn't clip images).
fn round_corners(mut img: RgbaImage, radius: f32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    for y in 0..h {
        for x in 0..w {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let cx = px.clamp(r, w as f32 - r);
            let cy = py.clamp(r, h as f32 - r);
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
