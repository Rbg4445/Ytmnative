<div align="center">
  <img src="assets/hero-banner.svg" alt="YTM Native v1.0.0" width="100%" />

  <br/>

  <p>
    <a href="https://github.com/Rbg4445/Ytmnative/releases/latest">
      <img src="https://img.shields.io/badge/Windows-YTM--Native--Setup--1.0.0--beta.exe-FF0033?style=for-the-badge&logo=windows&logoColor=white" alt="Download Windows Setup" />
    </a>
    <a href="https://github.com/Rbg4445/Ytmnative/releases/latest">
      <img src="https://img.shields.io/badge/Linux-.deb%20%26%20.tar.gz-FCC624?style=for-the-badge&logo=linux&logoColor=black" alt="Download Linux Package" />
    </a>
    <img src="https://img.shields.io/badge/Release-v1.0.0--beta-111116?style=for-the-badge&logo=github" alt="v1.0.0-beta" />
    <img src="https://img.shields.io/badge/Rust-Pure%20Audio%20Engine-DEA584?style=for-the-badge&logo=rust&logoColor=black" alt="Rust" />
    <img src="https://img.shields.io/badge/Background%20RAM-~12%20MB-10B981?style=for-the-badge" alt="~12 MB RAM" />
    <img src="https://img.shields.io/badge/Built%20with%20AI-Antigravity%20%26%20Claude-8B5CF6?style=for-the-badge" alt="Built with AI" />
    <a href="https://www.virustotal.com/gui/file/d332c2d4a3a239cee1f2785419b556d70513bac7b719555ecc29017beca692c3/detection">
      <img src="https://img.shields.io/badge/VirusTotal-Verify%20SHA--256-394EFF?style=for-the-badge&logo=virustotal&logoColor=white" alt="VirusTotal" />
    </a>
  </p>

  <p>
    <strong>An ultra-lightweight, pure-Rust desktop client for YouTube Music — zero Electron bloat, ~12 MB background RAM, and a frosted glass UI.</strong>
  </p>
</div>

---

## ✨ Features

<div align="center">
  <img src="assets/features-poster.svg" alt="YTM Native Features" width="100%" />
</div>

- **⚡ Instant Playback (~350 ms):** Pure-Rust Innertube HLS stream resolver, hover prefetching, and automatic next-2-tracks preloading.
- **🔍 Live Search Suggestions:** Real-time YouTube autocomplete dropdown with full keyboard navigation (`↑` / `↓` / `Enter` / `Esc`).
- **↻ Smart Refresh:** One-click **Refresh** button (`F5` / `Ctrl+R`) that rotates seed tracks from your listening history and likes to deliver genuinely fresh mixes.
- **🎧 High-Quality Rust Audio DSP:** Native `symphonia` AAC decoding + `cpal` (WASAPI) output with 4-point *Cubic Catmull-Rom* resampling and loudness normalization.
- **🪟 System Tray RAM Saver:** Closing the window (`X`) completely destroys WebView2 from memory while the pure-Rust audio engine, auto-radio, and Windows media keys (SMTC) keep running at **~12 MB Private RAM** and **0% idle CPU**.
- **🔐 Native Google Sign-In:** Sign in directly inside the app with zero Firebase or external browser cookie scraping to sync your YouTube Music history, likes, and picks.
- **🎮 Discord Rich Presence:** Native zero-dependency Discord IPC shows your current song, artist, album art, live timestamps, and a *"Listen on YouTube Music"* button on Discord.
- **🎤 Time-Synced Karaoke Lyrics (LRCLIB):** Real-time synchronized LRC lyrics with glowing active-line highlighting, auto-scroll, and click-any-line-to-seek.
- **🎥 OBS Browser Source Overlays & Stream Deck API (`:13091`):** Built-in local companion server at `http://127.0.0.1:13091/embed/now-playing` (`?layout=default|compact|stack|badge|text`) plus `/api/v1/state` and `/api/v1/command`.
- **📻 Last.fm Scrobbler & Now Playing:** Automatic Now Playing updates and track scrobbling at 50% playback.
- **⧉ Mini Player, Always-on-Top Pin & Theme Presets:** Floating `Ctrl+M` mini player with live synced lyrics, 6 glassmorphic theme presets, Custom CSS, and `ytmd://` shared-link & Tampermonkey userscript support.
- **🌐 Multi-Language Support (10 Languages):** Defaults to **English**, with instant switching in Settings (`English`, `Türkçe`, `Deutsch`, `Español`, `Français`, `Português`, `Русский`, `日本語`, `한국어`, `العربية`).
- **🚀 5-Second Serverless Auto-Updater:** Automatically checks GitHub Releases for up to 5 seconds on launch and self-updates seamlessly without requiring admin privileges.

---

## 📊 Memory & Performance

<div align="center">
  <img src="assets/ram-benchmark.svg" alt="Memory Benchmark" width="100%" />
</div>

---

## 🏗️ Architecture

<div align="center">
  <img src="assets/architecture-poster.svg" alt="Architecture Pipeline" width="100%" />
</div>

---

## 🛡️ Security & VirusTotal Verification

<div align="center">
  <a href="https://www.virustotal.com/gui/file/d332c2d4a3a239cee1f2785419b556d70513bac7b719555ecc29017beca692c3/detection">
    <img src="assets/security-poster.svg" alt="Security and VirusTotal Verification" width="100%" />
  </a>
</div>

| File | Windows Defender | SHA-256 | VirusTotal |
| :--- | :--- | :--- | :--- |
| `YTM-Native-Setup-1.0.0-beta.exe` | ✅ 0 Threats | `d332c2d4a3a239cee1f2785419b556d70513bac7b719555ecc29017beca692c3` | [VirusTotal Report](https://www.virustotal.com/gui/file/d332c2d4a3a239cee1f2785419b556d70513bac7b719555ecc29017beca692c3/detection) |
| `ytm-native.exe` | ✅ 0 Threats | `35299504705936268c40419122ed75dfd38b94d754b28cd49568486883d69eb8` | [VirusTotal Report](https://www.virustotal.com/gui/file/35299504705936268c40419122ed75dfd38b94d754b28cd49568486883d69eb8) |

---

## 📥 Installation

### Windows
1. Download **`YTM-Native-Setup-1.0.0-beta.exe`** (or portable **`YTM-Native-1.0.0-beta-windows-x64.zip`**) from the **[Releases](https://github.com/Rbg4445/Ytmnative/releases/latest)** page.
2. Run the installer (no admin/UAC required — installs per-user into `%LOCALAPPDATA%\YTM Native`).
3. Future updates are checked automatically on startup and installed seamlessly.

### Linux (Debian / Ubuntu / Mint / Arch / Fedora)
```bash
# Debian / Ubuntu / Linux Mint (.deb package)
sudo apt install ./ytm-native_1.0.0-beta_amd64.deb

# Universal Linux Tarball (.tar.gz)
tar -xzf YTM-Native-1.0.0-beta-linux-x64.tar.gz
./ytm-native
```

### Build Linux Package with Docker
```bash
docker build -f Dockerfile.linux -t ytm-native-linux .
docker create --name ytm-linux-extract ytm-native-linux
docker cp ytm-linux-extract:/out/. ./dist/
docker rm ytm-linux-extract
```

---

## ⌨️ Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| `Space` | Play / Pause |
| `Ctrl + K` or `/` | Focus search bar |
| `F5` or `Ctrl + R` | Refresh recommendations |
| `Ctrl + →` / `Ctrl + ←` | Next / Previous track |
| `→` / `←` | Seek forward / backward 5 seconds |
| Media Keys | Full hardware media key support via Windows SMTC |

---

## 🛠️ Building from Source

```powershell
# 1. Build release binary
cargo build --release

# 2. Build Windows Setup installer (Inno Setup 6)
iscc installer/ytm-native.iss
```

### Publishing a Release (Automated CI/CD)

Update the version in `Cargo.toml`, `tauri.conf.json`, and `installer/ytm-native.iss`, then push a tag — GitHub Actions automatically builds and publishes both `YTM-Native-Setup-*.exe` and `ytm-native.exe`:

```powershell
git tag v1.0.0
git push origin v1.0.0
```

---

## 🤝 Contributors & AI-Assisted Development

> **Built with AI:** This project was designed, architected, and built by **[@Rbg4445](https://github.com/Rbg4445)** using AI-powered software engineering with **Google Antigravity** and **Claude (Anthropic)**.

<table>
  <tr>
    <td align="center" width="170">
      <a href="https://github.com/Rbg4445">
        <img src="https://avatars.githubusercontent.com/u/217843180?v=4" width="68" height="68" style="border-radius:50%" alt="Rbg4445" /><br />
        <sub><b>@Rbg4445</b></sub>
      </a><br />
      <sub>Creator &amp; Maintainer</sub>
    </td>
    <td align="center" width="170">
      <a href="https://antigravity.google">
        <img src="assets/antigravity-logo.svg" width="68" height="68" alt="Google Antigravity" /><br />
        <sub><b>Google Antigravity</b></sub>
      </a><br />
      <sub>AI Agentic IDE</sub>
    </td>
    <td align="center" width="170">
      <a href="https://claude.ai">
        <img src="assets/claude-logo.svg" width="68" height="68" alt="Claude" /><br />
        <sub><b>Claude</b></sub>
      </a><br />
      <sub>AI Coding Assistant</sub>
    </td>
  </tr>
</table>

---

## ⚖️ Disclaimer & License

- **YTM Native** is an independent, unofficial open-source desktop client.
- It is **not affiliated with, endorsed by, or associated with Google LLC or YouTube**.
- "YouTube" and "YouTube Music" are registered trademarks of Google LLC.
- This application does not host or distribute any audio files; all streams are played directly on the user's device from publicly available or user-authenticated sessions.
- Released under the [MIT License](LICENSE).
