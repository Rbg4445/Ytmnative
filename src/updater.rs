use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use reqwest::header::{ACCEPT, USER_AGENT};
use serde::{Deserialize, Serialize};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default GitHub repo ("owner/repo") for VDS-free updates via GitHub Releases.
/// Can be overridden in the UI (saved in library.json).
pub const DEFAULT_GITHUB_REPO: &str = "Rbg4445/Ytmnative";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub release_notes: String,
    pub download_url: String,
    pub asset_name: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    body: Option<String>,
    html_url: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Remove leftover `.old` executable from a previous self-update.
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let old = old_exe_path(&exe);
        if old.exists() {
            let _ = fs::remove_file(old);
        }
    }
}

fn old_exe_path(exe: &std::path::Path) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(".old");
    PathBuf::from(name)
}

fn new_exe_path(exe: &std::path::Path) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(".new");
    PathBuf::from(name)
}

/// Parse `"v1.2.3"` or `"1.2.3"` into `(1, 2, 3)`.
pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let clean = s.trim().trim_start_matches(['v', 'V']);
    let core = clean.split('-').next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(remote: &str, local: &str) -> bool {
    match (parse_semver(remote), parse_semver(local)) {
        (Some(r), Some(l)) => r > l,
        _ => false,
    }
}

/// Check GitHub Releases (`https://api.github.com/repos/{repo}/releases/latest`) for a newer binary.
/// Strictly capped at a 5-second timeout so startup checks never hang when offline.
pub async fn check_for_update(client: &reqwest::Client, repo: &str) -> Result<Option<UpdateInfo>> {
    let trimmed = repo.trim().trim_matches('/');
    let repo = if trimmed.is_empty() {
        DEFAULT_GITHUB_REPO
    } else {
        trimmed
    };
    if repo.is_empty() || !repo.contains('/') {
        return Err(anyhow!(
            "GitHub deposu ayarlanmadı (örn: kullanici/ytm-native)"
        ));
    }

    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(5))
        .header(USER_AGENT, format!("ytm-native/{CURRENT_VERSION}"))
        .header(ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .context("GitHub Releases API isteği zaman aşımına uğradı veya başarısız oldu")?;

    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API HTTP {}", resp.status()));
    }

    let release: GithubRelease = resp
        .json()
        .await
        .context("GitHub release JSON ayrıştırılamadı")?;

    if !is_newer_version(&release.tag_name, CURRENT_VERSION) {
        return Ok(None);
    }

    // Prefer the standalone app binary (ytm-native.exe), never the Setup installer
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case("ytm-native.exe"))
        .or_else(|| {
            release.assets.iter().find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.ends_with(".exe") && !n.contains("setup") && !n.contains("installer")
            })
        })
        .or_else(|| release.assets.iter().find(|a| a.name.to_ascii_lowercase().ends_with(".exe")))
        .or_else(|| release.assets.first())
        .ok_or_else(|| anyhow!("Yeni sürümde ({}) indirilebilir dosya bulunamadı", release.tag_name))?;

    Ok(Some(UpdateInfo {
        version: release.tag_name,
        release_notes: release.body.unwrap_or_default(),
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        html_url: release.html_url,
    }))
}

/// Download the new executable and atomically replace the running binary, then restart.
pub async fn download_and_install(client: &reqwest::Client, info: &UpdateInfo) -> Result<()> {
    let current_exe = std::env::current_exe().context("Mevcut exe yolu alınamadı")?;
    let new_path = new_exe_path(&current_exe);
    let old_path = old_exe_path(&current_exe);

    let resp = client
        .get(&info.download_url)
        .timeout(std::time::Duration::from_secs(120))
        .header(USER_AGENT, format!("ytm-native/{CURRENT_VERSION}"))
        .send()
        .await
        .context("Güncelleme dosyası indirilemedi")?;

    if !resp.status().is_success() {
        return Err(anyhow!("İndirme hatası: HTTP {}", resp.status()));
    }

    let bytes = resp.bytes().await.context("İndirilen veri okunamadı")?;
    if bytes.len() < 100_000 {
        return Err(anyhow!("İndirilen dosya geçersiz görünüyor (çok küçük)"));
    }

    fs::write(&new_path, &bytes).context("Yeni sürüm diske yazılamadı")?;

    // On Windows, a running .exe can be renamed even while executing.
    if old_path.exists() {
        let _ = fs::remove_file(&old_path);
    }
    fs::rename(&current_exe, &old_path).context("Mevcut exe yedeklenemedi")?;

    if let Err(e) = fs::rename(&new_path, &current_exe) {
        // Roll back on failure
        let _ = fs::rename(&old_path, &current_exe);
        return Err(anyhow!("Yeni exe yerine taşınamadı: {e}"));
    }

    // Spawn the newly installed version
    Command::new(&current_exe)
        .spawn()
        .context("Yeni sürüm başlatılamadı")?;

    Ok(())
}
