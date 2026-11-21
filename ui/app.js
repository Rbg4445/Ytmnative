const invoke = window.__TAURI__.core.invoke;
const DEFAULT_UPDATE_REPO = "Rbg4445/Youtube-music-native";

const MOODS = [
  ["Enerjik", "enerjik müzik mix"],
  ["Keyifli", "keyifli pop şarkılar"],
  ["Rahatlama", "rahatlatıcı chill müzik"],
  ["Antrenman", "workout gym music"],
  ["İşe gidip gelme", "yol şarkıları mix"],
  ["Hüzünlü", "hüzünlü akustik şarkılar"],
  ["Parti", "parti dans hitleri"],
  ["Romantik", "romantik slow şarkılar"],
  ["Odaklanma", "odaklanma lofi instrumental"],
  ["Uyku", "uyku ambient müzik"],
];

let state = {
  page: "home",
  playlistId: null,
  sidebarExpanded: true,
  queueOpen: false,
  drawerTab: "queue",
  lyricsTrackId: null,
  lyricsData: null,
  loadingLyrics: false,
  mutedPrevVol: 0.85,
  activeMood: null,
  moodTracks: [],
  loadingMood: false,
  searchQuery: "",
  searchResults: [],
  searching: false,
  ytConnected: false,
  ytHistory: [],
  recommendations: [],
  loadingHome: true,
  homeRefreshNonce: 0,
  library: {
    recently_played: [],
    liked_tracks: [],
    playlists: [],
    volume: 0.85,
    update_repo: "",
    custom_css: "",
  },
  playback: {
    current: null,
    queue: [],
    current_index: null,
    is_playing: false,
    is_paused: false,
    is_buffering: false,
    position_secs: 0,
    duration_secs: 0,
    volume: 0.85,
    repeat: "off",
    shuffle: false,
    sleep_remaining_secs: null,
    sleep_after_track: false,
    stream_meta: null,
    last_error: null,
  },
  updateStatus: { kind: "idle", version: "1.0.0", info: null, message: "" },
  authMsg: "",
  authErr: "",
  authBusy: false,
};

let pollTimer = null;
let hoverPrefetchTimer = null;
const prefetchedIds = new Set();

function schedulePrefetch(trackId) {
  if (!trackId || prefetchedIds.has(trackId)) return;
  if (hoverPrefetchTimer) clearTimeout(hoverPrefetchTimer);
  hoverPrefetchTimer = setTimeout(() => {
    prefetchedIds.add(trackId);
    invoke("prefetch_track", { trackId }).catch(() => {});
  }, 280);
}

function cancelPrefetch() {
  if (hoverPrefetchTimer) {
    clearTimeout(hoverPrefetchTimer);
    hoverPrefetchTimer = null;
  }
}

function esc(s) {
  return String(s ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function fmtSec(sec) {
  const total = Math.max(0, Math.round(sec || 0));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0
    ? `${h}:${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`
    : `${m}:${String(s).padStart(2, "0")}`;
}

function isLiked(trackId) {
  return state.library.liked_tracks.some((t) => t.id === trackId);
}

function showBanner(msg, isError = false) {
  const el = document.getElementById("status-banner");
  if (!msg) {
    el.classList.add("hidden");
    return;
  }
  el.textContent = msg;
  el.className = isError
    ? "mx-8 mb-2 px-3.5 py-2 rounded-lg text-xs bg-[#3a0d16] text-red-200 border border-[#FF0033]/40"
    : "mx-8 mb-2 px-3.5 py-2 rounded-lg text-xs bg-white/10 text-neutral-200";
  el.classList.remove("hidden");
}

function navigate(page, playlistId = null) {
  state.page = page;
  state.playlistId = playlistId;
  renderNav();
  renderMain();
}

function toggleQueue() {
  state.queueOpen = !state.queueOpen;
  const drawer = document.getElementById("queue-drawer");
  const btn = document.getElementById("btn-toggle-queue");
  if (state.queueOpen) {
    drawer.classList.remove("hidden");
    drawer.classList.add("flex");
    btn.classList.add("text-[#FF0033]");
    renderQueue();
  } else {
    drawer.classList.add("hidden");
    drawer.classList.remove("flex");
    btn.classList.remove("text-[#FF0033]");
  }
}

function toggleNewPlaylistBox() {
  const box = document.getElementById("new-playlist-box");
  box.classList.toggle("hidden");
  if (!box.classList.contains("hidden")) {
    document.getElementById("new-playlist-input").focus();
  }
}

async function createPlaylist() {
  const input = document.getElementById("new-playlist-input");
  const name = input.value.trim();
  input.value = "";
  document.getElementById("new-playlist-box").classList.add("hidden");
  const id = await invoke("create_playlist", { name });
  await refreshLibrary();
  navigate("playlist", id);
}

function renderNav() {
  document.querySelectorAll(".nav-item").forEach((btn) => {
    const active = btn.dataset.nav === state.page;
    btn.className = active
      ? "nav-item w-full flex items-center gap-4 px-3 py-2.5 rounded-lg text-[14.5px] font-medium bg-[#212121] text-white transition cursor-pointer"
      : "nav-item w-full flex items-center gap-4 px-3 py-2.5 rounded-lg text-[14.5px] font-medium text-neutral-300 hover:bg-white/[0.08] hover:text-white transition cursor-pointer";
  });

  // Account avatar state
  const av = document.getElementById("btn-account-avatar");
  if (state.ytConnected) {
    av.className =
      "w-8 h-8 rounded-full bg-[#FF0033] ring-2 ring-[#FF0033]/40 flex items-center justify-center text-xs font-bold text-white transition cursor-pointer";
    av.textContent = "✓";
    av.title = "YouTube Hesabı Bağlı";
  } else {
    av.className =
      "w-8 h-8 rounded-full bg-neutral-500 hover:ring-2 hover:ring-white/40 flex items-center justify-center text-xs font-bold text-white transition cursor-pointer";
    av.textContent = "";
    av.title = "YouTube Hesabı Bağla (Geçmiş & Öneriler)";
  }

  // Updater pill
  const upd = document.getElementById("btn-updater-pill");
  if (state.updateStatus.kind === "available") {
    upd.textContent = `⬆ ${state.updateStatus.info.version} Güncelleme Hazır`;
    upd.className =
      "mt-2 pt-2 border-t border-white/10 text-left px-2 py-1 text-[11.5px] font-medium text-[#FF0033] hover:text-white transition cursor-pointer truncate";
  } else {
    upd.textContent = `v${state.updateStatus.version} • Güncelleme`;
    upd.className =
      "mt-2 pt-2 border-t border-white/10 text-left px-2 py-1 text-[11.5px] text-neutral-400 hover:text-white transition cursor-pointer truncate";
  }

  // Sidebar playlists
  const listEl = document.getElementById("sidebar-playlists");
  const likedActive = state.page === "liked";
  let html = `
    <button onclick="navigate('liked')" class="w-full text-left px-2.5 py-2 rounded-lg transition cursor-pointer ${
      likedActive ? "bg-[#212121]" : "hover:bg-white/[0.08]"
    }">
      <div class="text-[13.5px] font-medium text-white truncate">Beğendiğim müzikler</div>
      <div class="text-[11.5px] text-neutral-400 truncate mt-0.5">📌 Otomatik oynatma listesi • ${
        state.library.liked_tracks.length
      }</div>
    </button>
  `;
  for (const pl of state.library.playlists) {
    const active = state.page === "playlist" && state.playlistId === pl.id;
    html += `
      <button onclick="navigate('playlist', '${esc(pl.id)}')" class="w-full text-left px-2.5 py-2 rounded-lg transition cursor-pointer ${
        active ? "bg-[#212121]" : "hover:bg-white/[0.08]"
      }">
        <div class="text-[13.5px] font-medium text-white truncate">${esc(pl.name)}</div>
        <div class="text-[11.5px] text-neutral-400 truncate mt-0.5">Oynatma listesi • ${pl.tracks.length}</div>
      </button>
    `;
  }
  listEl.innerHTML = html;
}

function renderCardCarousel(id, tracks) {
  const curId = state.playback.current?.id;
  return `
    <div id="${id}" class="flex gap-5 overflow-x-auto no-scrollbar scroll-smooth pb-2">
      ${tracks
        .map((t) => {
          const active = curId === t.id;
          const safeJson = esc(JSON.stringify(t));
          return `
            <div onclick='playTrackJson(${safeJson})' onmouseenter="schedulePrefetch('${esc(t.id)}')" onmouseleave="cancelPrefetch()" class="group w-[184px] shrink-0 cursor-pointer">
              <div class="relative w-[184px] h-[184px] rounded-xl overflow-hidden bg-white/[0.05] border border-white/[0.08] shadow-lg group-hover:border-white/25 transition">
                ${
                  t.thumbnail_url
                    ? `<img src="${esc(t.thumbnail_url)}" loading="lazy" alt="" class="w-full h-full object-cover group-hover:scale-[1.04] transition duration-300" />`
                    : ""
                }
                <div class="absolute inset-0 bg-black/20 group-hover:bg-black/45 transition flex items-center justify-center">
                  <div class="w-12 h-12 rounded-full bg-black/45 backdrop-blur-md border border-white/20 flex items-center justify-center shadow-xl transition-transform group-hover:scale-110">
                    <svg class="w-6 h-6 ml-0.5 ${
                      active ? "text-[#FF0033]" : "text-white"
                    }" fill="currentColor" viewBox="0 0 24 24">
                      <path d="M8 5v14l11-7z"/>
                    </svg>
                  </div>
                </div>
              </div>
              <div class="mt-2.5 text-[14px] font-semibold leading-snug line-clamp-2 ${
                active ? "text-[#FF0033]" : "text-white"
              }">${esc(t.title)}</div>
              <div class="mt-0.5 text-[13px] text-neutral-400 line-clamp-2">${esc(t.artist)}</div>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function scrollCarousel(id, delta) {
  const el = document.getElementById(id);
  if (el) el.scrollBy({ left: delta, behavior: "smooth" });
}

function renderTrackRows(tracks, opts = {}) {
  const curId = state.playback.current?.id;
  return `
    <div class="divide-y divide-white/[0.06]">
      ${tracks
        .map((t, idx) => {
          const active = curId === t.id;
          const liked = isLiked(t.id);
          const safeTrack = esc(JSON.stringify(t));
          const playCall = opts.collection
            ? `playCollectionIndex(${idx})`
            : `playTrackJson(${safeTrack})`;
          return `
            <div class="group flex items-center gap-3.5 py-2.5 px-2 rounded-md hover:bg-white/[0.07] transition ${
              active ? "bg-white/[0.09]" : ""
            }">
              <div onclick='${playCall}' class="relative w-11 h-11 rounded overflow-hidden bg-[#181818] shrink-0 cursor-pointer">
                ${
                  t.thumbnail_url
                    ? `<img src="${esc(t.thumbnail_url)}" loading="lazy" alt="" class="w-full h-full object-cover" />`
                    : ""
                }
                <div class="absolute inset-0 bg-black/40 ${
                  active ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                } flex items-center justify-center transition">
                  <svg class="w-5 h-5 ${active ? "text-[#FF0033]" : "text-white"}" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>
                </div>
              </div>
              <div onclick='${playCall}' class="flex-1 min-w-0 cursor-pointer">
                <div class="text-[14px] font-medium truncate ${
                  active ? "text-[#FF0033]" : "text-white"
                }">${esc(t.title)}</div>
                <div class="text-[13px] text-neutral-400 truncate">${esc(t.artist)}</div>
              </div>
              <div class="flex items-center gap-3 shrink-0">
                <button onclick='toggleLikeJson(${safeTrack})' title="Beğen" class="p-1.5 rounded-full hover:bg-white/10 transition cursor-pointer ${
                  liked ? "text-[#FF0033]" : "text-neutral-400 hover:text-white"
                }">
                  <svg class="w-4 h-4" fill="${liked ? "currentColor" : "none"}" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78Z"/></svg>
                </button>
                ${
                  opts.playlistId
                    ? `<button onclick="removeFromPlaylist('${esc(opts.playlistId)}', '${esc(t.id)}')" title="Listeden çıkar" class="text-xs text-neutral-400 hover:text-white px-1.5 cursor-pointer">✕</button>`
                    : state.library.playlists.length > 0
                    ? `<select onchange="addToPlaylistSelect(this, ${safeTrack})" class="bg-[#1b1b1b] border border-white/10 text-[11px] text-neutral-300 rounded px-1.5 py-1 cursor-pointer focus:outline-none">
                        <option value="">+ Liste</option>
                        ${state.library.playlists
                          .map((p) => `<option value="${esc(p.id)}">${esc(p.name)}</option>`)
                          .join("")}
                      </select>`
                    : ""
                }
                <span class="text-xs text-neutral-400 tabular-nums w-10 text-right">${
                  t.duration_secs ? fmtSec(t.duration_secs) : "--:--"
                }</span>
              </div>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

let currentCollectionTracks = [];

function renderMain() {
  const main = document.getElementById("main-content");

  if (state.page === "home") {
    const recent =
      state.ytHistory.length > 0 ? state.ytHistory : state.library.recently_played;
    const recentEyebrow =
      state.ytHistory.length > 0 ? "YOUTUBE GEÇMİŞİN" : "SON DİNLEDİKLERİN";
    const recTitle =
      state.ytHistory.length > 0
        ? "YouTube geçmişine göre öneriler"
        : state.library.recently_played.length > 0
        ? "Sana özel öneriler"
        : "Hızlı seçimler";

    main.innerHTML = `
      <!-- MOOD CHIPS ROW -->
      <div class="flex items-center gap-2.5 overflow-x-auto no-scrollbar pb-1 mb-8">
        ${MOODS.map(([label, query]) => {
          const active = state.activeMood === label;
          return `
            <button onclick="selectMood('${esc(label)}', '${esc(query)}')" class="shrink-0 px-3.5 py-1.5 rounded-lg text-[13.5px] font-medium border transition cursor-pointer ${
              active
                ? "bg-white text-black border-white"
                : "bg-white/[0.10] hover:bg-white/[0.18] text-white border-white/[0.08]"
            }">${esc(label)}</button>
          `;
        }).join("")}
      </div>

      ${
        state.activeMood
          ? `
        <section class="mb-10">
          <div class="flex items-end justify-between mb-4">
            <div>
              <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">RUH HALİ</div>
              <h2 class="text-[28px] font-bold leading-tight">${esc(state.activeMood)} ${
                state.loadingMood ? '<span class="text-sm font-normal text-[#FF0033] ml-2">yükleniyor...</span>' : ""
              }</h2>
            </div>
            <button onclick="clearMood()" class="px-3.5 py-1.5 rounded-full border border-white/20 hover:bg-white/10 text-xs font-medium text-white cursor-pointer">Filtreyi temizle</button>
          </div>
          ${renderCardCarousel("carousel-mood", state.moodTracks)}
        </section>`
          : ""
      }

      ${
        recent.length > 0
          ? `
        <section class="mb-11">
          <div class="flex items-end justify-between mb-4">
            <div>
              <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">${recentEyebrow}</div>
              <h2 class="text-[28px] font-bold leading-tight">Yeniden dinleyin</h2>
            </div>
            <div class="flex items-center gap-2">
              <button onclick="scrollCarousel('carousel-recent', -420)" class="w-9 h-9 rounded-full border border-white/15 hover:bg-white/10 flex items-center justify-center text-lg text-white cursor-pointer">‹</button>
              <button onclick="scrollCarousel('carousel-recent', 420)" class="w-9 h-9 rounded-full border border-white/15 hover:bg-white/10 flex items-center justify-center text-lg text-white cursor-pointer">›</button>
            </div>
          </div>
          ${renderCardCarousel("carousel-recent", recent)}
        </section>`
          : ""
      }

      <section class="mb-8">
        <div class="flex items-end justify-between mb-4">
          <div>
            <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">${
              state.loadingHome ? "YÜKLENİYOR..." : "SENİN İÇİN SEÇTİKLERİMİZ"
            }</div>
            <h2 class="text-[28px] font-bold leading-tight">${recTitle}</h2>
          </div>
          <div class="flex items-center gap-2.5">
            <button onclick="refreshHome(true)" ${state.loadingHome ? "disabled" : ""} class="flex items-center gap-1.5 px-3.5 py-1.5 rounded-full border border-white/20 hover:bg-white/10 text-xs font-medium text-white transition cursor-pointer disabled:opacity-60">
              <svg class="w-3.5 h-3.5 ${state.loadingHome ? "animate-spin text-[#FF0033]" : ""}" fill="none" stroke="currentColor" stroke-width="2.2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M21 12a9 9 0 1 1-2.64-6.36L21 8m0-5v5h-5"/></svg>
              <span>${state.loadingHome ? "Yenileniyor..." : "Yenile"}</span>
            </button>
            <button onclick="scrollCarousel('carousel-rec', -420)" class="w-9 h-9 rounded-full border border-white/15 hover:bg-white/10 flex items-center justify-center text-lg text-white cursor-pointer">‹</button>
            <button onclick="scrollCarousel('carousel-rec', 420)" class="w-9 h-9 rounded-full border border-white/15 hover:bg-white/10 flex items-center justify-center text-lg text-white cursor-pointer">›</button>
          </div>
        </div>
        ${renderCardCarousel("carousel-rec", state.recommendations)}
      </section>
    `;
    return;
  }

  if (state.page === "explore") {
    main.innerHTML = `
      <h1 class="text-[30px] font-bold mb-6">Keşfet</h1>
      <h2 class="text-lg font-semibold mb-3 text-neutral-200">Ruh halleri ve türler</h2>
      <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-5 gap-3">
        ${MOODS.map(
          ([label, query]) => `
          <button onclick="navigate('home'); selectMood('${esc(label)}', '${esc(query)}')" class="h-12 px-4 rounded-lg bg-[#1d1d1d] hover:bg-[#2a2a2a] border-l-4 border-[#FF0033] text-left text-sm font-medium text-white transition cursor-pointer">
            ${esc(label)}
          </button>`
        ).join("")}
      </div>
    `;
    return;
  }

  if (state.page === "library") {
    main.innerHTML = `
      <h1 class="text-[30px] font-bold mb-6">Kitaplık</h1>
      <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-5">
        <div onclick="navigate('liked')" class="group cursor-pointer">
          <div class="w-full aspect-square rounded-md bg-gradient-to-br from-[#FF0033] to-[#4a0817] flex items-center justify-center text-5xl text-white shadow-lg group-hover:scale-[1.02] transition">♥</div>
          <div class="mt-2.5 text-sm font-semibold text-white">Beğendiğim müzikler</div>
          <div class="text-xs text-neutral-400">Otomatik oynatma listesi • ${state.library.liked_tracks.length}</div>
        </div>
        ${state.library.playlists
          .map((pl) => {
            const cover = pl.tracks[0]?.thumbnail_url;
            return `
              <div onclick="navigate('playlist', '${esc(pl.id)}')" class="group cursor-pointer">
                <div class="w-full aspect-square rounded-md bg-[#1a1a1a] overflow-hidden flex items-center justify-center text-4xl text-neutral-600 group-hover:scale-[1.02] transition">
                  ${cover ? `<img src="${esc(cover)}" class="w-full h-full object-cover" />` : "♫"}
                </div>
                <div class="mt-2.5 text-sm font-semibold text-white truncate">${esc(pl.name)}</div>
                <div class="text-xs text-neutral-400">Oynatma listesi • ${pl.tracks.length} parça</div>
              </div>
            `;
          })
          .join("")}
      </div>
    `;
    return;
  }

  if (state.page === "search") {
    main.innerHTML = `
      <h1 class="text-2xl font-bold mb-4">Arama sonuçları ${
        state.searching ? '<span class="text-sm font-normal text-[#FF0033] ml-2">aranıyor...</span>' : ""
      }</h1>
      ${
        state.searchResults.length > 0
          ? renderTrackRows(state.searchResults)
          : `<p class="text-sm text-neutral-400">Üstteki arama çubuğuna bir şarkı veya sanatçı yazıp Enter'a bas.</p>`
      }
    `;
    return;
  }

  if (state.page === "liked") {
    currentCollectionTracks = state.library.liked_tracks;
    main.innerHTML = `
      <div class="flex items-end justify-between mb-6 pb-4 border-b border-white/10">
        <div>
          <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">OTOMATİK OYNATMA LİSTESİ</div>
          <h1 class="text-[30px] font-bold">Beğendiğim müzikler</h1>
          <div class="text-xs text-neutral-400 mt-1">${currentCollectionTracks.length} parça</div>
        </div>
        ${
          currentCollectionTracks.length > 0
            ? `<button onclick="playCollectionIndex(0)" class="px-5 py-2.5 rounded-full bg-white hover:bg-neutral-200 text-black text-sm font-semibold cursor-pointer">▶  Tümünü oynat</button>`
            : ""
        }
      </div>
      ${
        currentCollectionTracks.length > 0
          ? renderTrackRows(currentCollectionTracks, { collection: true })
          : `<p class="text-sm text-neutral-400">Henüz beğendiğin şarkı yok.</p>`
      }
    `;
    return;
  }

  if (state.page === "playlist") {
    const pl = state.library.playlists.find((p) => p.id === state.playlistId);
    if (!pl) {
      navigate("home");
      return;
    }
    currentCollectionTracks = pl.tracks;
    main.innerHTML = `
      <div class="flex items-end justify-between mb-6 pb-4 border-b border-white/10">
        <div>
          <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">OYNATMA LİSTESİ</div>
          <h1 class="text-[30px] font-bold">${esc(pl.name)}</h1>
          <div class="text-xs text-neutral-400 mt-1">${pl.tracks.length} parça</div>
        </div>
        <div class="flex items-center gap-2.5">
          ${
            pl.tracks.length > 0
              ? `<button onclick="playCollectionIndex(0)" class="px-5 py-2 rounded-full bg-white hover:bg-neutral-200 text-black text-sm font-semibold cursor-pointer">▶  Oynat</button>`
              : ""
          }
          <button onclick="deletePlaylist('${esc(pl.id)}')" class="px-4 py-2 rounded-full border border-white/20 hover:bg-[#FF0033]/20 text-xs font-medium text-[#FF0033] cursor-pointer">Sil</button>
        </div>
      </div>
      ${
        pl.tracks.length > 0
          ? renderTrackRows(pl.tracks, { collection: true, playlistId: pl.id })
          : `<p class="text-sm text-neutral-400">Bu liste boş. Arama ekranından "+ Liste" ile parça ekleyebilirsin.</p>`
      }
    `;
    return;
  }

  if (state.page === "account") {
    main.innerHTML = `
      <h1 class="text-[28px] font-bold mb-2">YouTube Music Hesabı</h1>
      <p class="text-sm text-neutral-400 mb-6">Hesabını bağladığında "Yeniden dinleyin", beğendiklerin ve öneriler doğrudan YouTube geçmişinden çekilir.</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-6 space-y-5">
        ${
          state.ytConnected
            ? `
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2.5">
                <span class="w-2.5 h-2.5 rounded-full bg-[#FF0033]"></span>
                <span class="font-semibold text-white">YouTube Music Hesabı Bağlı</span>
              </div>
              <div class="flex gap-2">
                <button onclick="refreshHome()" class="px-4 py-2 rounded-full bg-white text-black text-xs font-semibold cursor-pointer">Geçmişi Eşitle</button>
                <button onclick="disconnectYt()" class="px-4 py-2 rounded-full border border-white/20 text-xs text-neutral-300 hover:text-white cursor-pointer">Bağlantıyı Kes</button>
              </div>
            </div>`
            : `
            <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              <div>
                <h3 class="text-base font-semibold text-white">Google Hesabınla Oturum Aç</h3>
                <p class="text-xs text-neutral-400 mt-1">Resmi Google giriş penceresi açılır; giriş yaptığında YouTube geçmişin, beğendiklerin ve önerilerin otomatik eşitlenir.</p>
              </div>
              <button ${state.authBusy ? "disabled" : ""} onclick="loginWithGoogle()" class="shrink-0 flex items-center justify-center gap-3 px-5 py-2.5 rounded-full bg-white hover:bg-neutral-100 disabled:opacity-50 text-[#1f1f1f] text-sm font-semibold shadow-lg transition cursor-pointer">
                <svg class="w-4 h-4 shrink-0" viewBox="0 0 24 24">
                  <path fill="#4285F4" d="M23.49 12.27c0-.79-.07-1.54-.2-2.27H12v4.51h6.47a5.54 5.54 0 0 1-2.4 3.63v3.01h3.88c2.27-2.09 3.54-5.17 3.54-8.88Z"/>
                  <path fill="#34A853" d="M12 24c3.24 0 5.95-1.08 7.94-2.91l-3.88-3.01c-1.08.72-2.45 1.16-4.06 1.16-3.12 0-5.77-2.11-6.72-4.96H1.29v3.11A12 12 0 0 0 12 24Z"/>
                  <path fill="#FBBC05" d="M5.28 14.28A7.22 7.22 0 0 1 4.9 12c0-.79.14-1.56.38-2.28V6.61H1.29A12 12 0 0 0 0 12c0 1.94.46 3.77 1.29 5.39l3.99-3.11Z"/>
                  <path fill="#EA4335" d="M12 4.76c1.76 0 3.34.61 4.59 1.8l3.44-3.44C17.95 1.19 15.24 0 12 0A12 12 0 0 0 1.29 6.61l3.99 3.11c.95-2.85 3.6-4.96 6.72-4.96Z"/>
                </svg>
                <span>${state.authBusy ? "Giriş bekleniyor..." : "Google ile Oturum Aç"}</span>
              </button>
            </div>`
        }
        ${state.authMsg ? `<div class="text-xs text-emerald-400">${esc(state.authMsg)}</div>` : ""}
        ${state.authErr ? `<div class="text-xs text-[#FF0033]">⚠ ${esc(state.authErr)}</div>` : ""}
      </div>

      <h2 class="text-xl font-bold mt-8 mb-2">Uygulama Ayarları</h2>
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between mb-6">
        <div>
          <div class="text-sm font-semibold text-white">Açılış İntro Videosu</div>
          <div class="text-xs text-neutral-400 mt-0.5">Yalnızca ilk açılışta otomatik oynatılır; dilediğinde buradan tekrar izleyebilirsin.</div>
        </div>
        <button onclick="showIntro(false)" class="shrink-0 px-4 py-2 rounded-full bg-white/10 hover:bg-[#FF0033] border border-white/15 hover:border-[#FF0033] text-xs font-semibold text-white transition cursor-pointer">🎬 İntroyu Göster</button>
      </div>

      <h2 class="text-xl font-bold mb-2">Özel CSS (Custom CSS)</h2>
      <p class="text-xs text-neutral-400 mb-3">Arayüzü kendi stilinle özelleştir (ytmdesktop özelliği — anında uygulanır ve kaydedilir).</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-5 space-y-3">
        <textarea id="custom-css-input" rows="4" placeholder="/* Örn: #player-bar { background: #141414 !important; } */" class="w-full font-mono bg-[#0a0a0a] border border-white/15 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-white/40">${esc(state.library.custom_css || "")}</textarea>
        <button onclick="saveCustomCss()" class="px-5 py-2 rounded-full bg-white hover:bg-neutral-200 text-black text-xs font-semibold cursor-pointer">CSS'i Uygula ve Kaydet</button>
      </div>
    `;
    return;
  }

  if (state.page === "updates") {
    const repo = state.library.update_repo || DEFAULT_UPDATE_REPO;
    const u = state.updateStatus;
    main.innerHTML = `
      <h1 class="text-[28px] font-bold mb-2">Güncelleme Sistemi (VDS Gerektirmez)</h1>
      <p class="text-sm text-neutral-400 mb-6">Mevcut sürüm: v${esc(u.version)} • Güncellemeler ücretsiz GitHub Releases üzerinden tek tıkla indirilir.</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-6 space-y-4">
        <div class="flex gap-2.5">
          <input id="repo-input" type="text" value="${esc(repo)}" placeholder="kullanici/ytm-native" class="flex-1 bg-[#0a0a0a] border border-white/15 rounded-lg px-3.5 py-2 text-sm text-white focus:outline-none focus:border-white/40" />
          <button onclick="saveAndCheckUpdate()" class="px-5 py-2 rounded-full bg-white hover:bg-neutral-200 text-black text-xs font-semibold cursor-pointer">Kaydet ve Kontrol Et</button>
        </div>
        ${
          u.kind === "checking"
            ? `<div class="text-xs text-neutral-300">⟳ GitHub Releases kontrol ediliyor...</div>`
            : u.kind === "uptodate"
            ? `<div class="text-xs text-emerald-400">✓ Uygulama tamamen güncel!</div>`
            : u.kind === "downloading"
            ? `<div class="text-xs text-[#FF0033]">⬇ Yeni sürüm indiriliyor ve kuruluyor...</div>`
            : u.kind === "error"
            ? `<div class="text-xs text-[#FF0033]">⚠ ${esc(u.message)}</div>`
            : u.kind === "available"
            ? `
              <div class="p-4 rounded-lg bg-[#211014] border border-[#FF0033]/40 space-y-2.5">
                <div class="text-sm font-semibold text-white">🎉 Yeni sürüm hazır: ${esc(u.info.version)} (${esc(u.info.asset_name)})</div>
                ${u.info.release_notes ? `<pre class="text-xs text-neutral-300 whitespace-pre-wrap">${esc(u.info.release_notes)}</pre>` : ""}
                <button onclick="installUpdate()" class="px-5 py-2 rounded-full bg-[#FF0033] hover:bg-[#e0002d] text-white text-xs font-semibold cursor-pointer">⬆ İndir, Güncelle ve Yeniden Başlat</button>
              </div>`
            : `<p class="text-xs text-neutral-400">Yeni sürüm yayınlamak için repoya <code>v0.2.0</code> etiketi atman yeterli; GitHub Actions Windows exe'sini otomatik derleyip yayınlar.</p>`
        }
      </div>
    `;
  }
}

function renderQueue() {
  if (!state.queueOpen) return;
  const list = document.getElementById("queue-list");
  const q = state.playback.queue || [];
  const curIdx = state.playback.current_index;
  if (q.length === 0) {
    list.innerHTML = `<p class="text-xs text-neutral-400">Bir şarkı başlattığında benzer parçalar burada sıralanır.</p>`;
    return;
  }
  list.innerHTML = q
    .map((t, i) => {
      const active = i === curIdx;
      return `
        <div onclick="selectQueueIndex(${i})" class="flex items-center gap-2.5 p-1.5 rounded-md cursor-pointer transition ${
          active ? "bg-white/15" : "hover:bg-white/[0.07]"
        }">
          <img src="${esc(t.thumbnail_url || "")}" loading="lazy" alt="" class="w-9 h-9 rounded object-cover bg-neutral-800 shrink-0" />
          <div class="min-w-0 flex-1">
            <div class="text-[13px] font-medium truncate ${active ? "text-[#FF0033]" : "text-white"}">${esc(t.title)}</div>
            <div class="text-[11.5px] text-neutral-400 truncate">${esc(t.artist)}</div>
          </div>
        </div>
      `;
    })
    .join("");
}

function switchDrawerTab(tab) {
  state.drawerTab = tab;
  const qTab = document.getElementById("tab-drawer-queue");
  const lTab = document.getElementById("tab-drawer-lyrics");
  const qList = document.getElementById("queue-list");
  const lPanel = document.getElementById("lyrics-panel");
  if (tab === "queue") {
    qTab.className = "text-xs font-bold tracking-wider uppercase text-white border-b-2 border-white pb-1 cursor-pointer";
    lTab.className = "text-xs font-bold tracking-wider uppercase text-neutral-400 hover:text-white pb-1 cursor-pointer";
    qList.classList.remove("hidden");
    lPanel.classList.add("hidden");
    renderQueue();
  } else {
    lTab.className = "text-xs font-bold tracking-wider uppercase text-white border-b-2 border-white pb-1 cursor-pointer";
    qTab.className = "text-xs font-bold tracking-wider uppercase text-neutral-400 hover:text-white pb-1 cursor-pointer";
    qList.classList.add("hidden");
    lPanel.classList.remove("hidden");
    loadLyricsForCurrent();
  }
}

async function loadLyricsForCurrent() {
  const cur = state.playback.current;
  const panel = document.getElementById("lyrics-panel");
  if (!cur) {
    panel.innerHTML = `<p class="text-xs text-neutral-400">Şarkı sözlerini görmek için bir parça çal.</p>`;
    return;
  }
  if (state.lyricsTrackId === cur.id && !state.loadingLyrics) {
    renderLyricsPanel();
    return;
  }
  state.lyricsTrackId = cur.id;
  state.lyricsData = null;
  state.loadingLyrics = true;
  panel.innerHTML = `<p class="text-xs text-neutral-400">Sözler yükleniyor...</p>`;
  try {
    state.lyricsData = await invoke("get_lyrics", { videoId: cur.id });
  } catch (_) {
    state.lyricsData = null;
  }
  state.loadingLyrics = false;
  if (state.lyricsTrackId === cur.id) {
    renderLyricsPanel();
  }
}

function renderLyricsPanel() {
  const panel = document.getElementById("lyrics-panel");
  if (!state.lyricsData || !state.lyricsData.body) {
    panel.innerHTML = `<p class="text-xs text-neutral-400">Bu parça için şarkı sözü bulunamadı.</p>`;
    return;
  }
  panel.innerHTML = `
    <div class="text-[14px] leading-7 text-neutral-100">${esc(state.lyricsData.body)}</div>
    ${state.lyricsData.footer ? `<div class="mt-6 text-[11px] text-neutral-500">${esc(state.lyricsData.footer)}</div>` : ""}
  `;
}

function renderPlayerBar() {
  const pb = state.playback;
  const bar = document.getElementById("player-bar");
  if (!pb.current && !pb.is_playing && !pb.is_buffering) {
    bar.classList.add("hidden");
    return;
  }
  bar.classList.remove("hidden");

  const dur = Math.max(1, pb.duration_secs || pb.current?.duration_secs || 1);
  const pos = Math.min(dur, pb.position_secs || 0);
  const pct = (pos / dur) * 100;

  document.getElementById("seek-bar-fill").style.width = `${pct}%`;
  document.getElementById("player-time").textContent = `${fmtSec(pos)} / ${fmtSec(dur)}`;

  const playIcon = document.getElementById("icon-play");
  const pauseIcon = document.getElementById("icon-pause");
  const loadIcon = document.getElementById("icon-loading");
  if (pb.is_buffering) {
    playIcon.classList.add("hidden");
    pauseIcon.classList.add("hidden");
    loadIcon.classList.remove("hidden");
  } else if (pb.is_playing) {
    playIcon.classList.add("hidden");
    loadIcon.classList.add("hidden");
    pauseIcon.classList.remove("hidden");
  } else {
    pauseIcon.classList.add("hidden");
    loadIcon.classList.add("hidden");
    playIcon.classList.remove("hidden");
  }

  if (pb.current) {
    document.getElementById("player-cover").src = pb.current.thumbnail_url || "";
    const amb = document.getElementById("ambient-bg-img");
    if (amb && pb.current.thumbnail_url) {
      if (amb.src !== pb.current.thumbnail_url) amb.src = pb.current.thumbnail_url;
      amb.classList.remove("hidden");
    }
    document.getElementById("player-title").textContent = pb.current.title;
    document.getElementById("player-artist").textContent = pb.current.artist;
    const liked = isLiked(pb.current.id);
    const likeBtn = document.getElementById("btn-player-like");
    likeBtn.className = liked
      ? "ml-2 text-[#FF0033] transition cursor-pointer"
      : "ml-2 text-neutral-400 hover:text-white transition cursor-pointer";
    likeBtn.querySelector("svg").setAttribute("fill", liked ? "currentColor" : "none");
  }

  // Shuffle & Repeat visual states
  const shufBtn = document.getElementById("btn-shuffle");
  shufBtn.className = pb.shuffle
    ? "p-1.5 rounded hover:bg-white/10 text-[#FF0033] transition cursor-pointer"
    : "p-1.5 rounded hover:bg-white/10 text-neutral-400 hover:text-white transition cursor-pointer";

  const repBtn = document.getElementById("btn-repeat");
  const repOne = document.getElementById("repeat-one-badge");
  repBtn.className =
    pb.repeat && pb.repeat !== "off"
      ? "relative p-1.5 rounded hover:bg-white/10 text-[#FF0033] transition cursor-pointer"
      : "relative p-1.5 rounded hover:bg-white/10 text-neutral-400 hover:text-white transition cursor-pointer";
  repOne.classList.toggle("hidden", pb.repeat !== "one");

  // Sleep Timer countdown badge
  const sleepBtn = document.getElementById("btn-sleep");
  if (pb.sleep_after_track) {
    sleepBtn.textContent = "🌙 Son";
    sleepBtn.className = "px-2 py-1 rounded bg-[#FF0033]/20 text-xs text-[#FF0033] font-medium cursor-pointer";
  } else if (pb.sleep_remaining_secs != null) {
    const mins = Math.ceil(pb.sleep_remaining_secs / 60);
    sleepBtn.textContent = `🌙 ${mins}dk`;
    sleepBtn.className = "px-2 py-1 rounded bg-[#FF0033]/20 text-xs text-[#FF0033] font-medium cursor-pointer";
  } else {
    sleepBtn.textContent = "🌙";
    sleepBtn.className = "px-2 py-1 rounded hover:bg-white/10 text-xs text-neutral-400 hover:text-white transition cursor-pointer";
  }

  document.getElementById("player-quality").textContent = pb.stream_meta || "";
  document.getElementById("volume-slider").value = pb.volume;

  if (pb.last_error) {
    showBanner(`⚠ ${pb.last_error}`, true);
  }
}

// --- BACKEND COMMAND WRAPPERS ---

async function syncPlayback() {
  const prevId = state.playback.current?.id;
  const prevQLen = state.playback.queue?.length || 0;
  state.playback = await invoke("get_playback_state");
  renderPlayerBar();
  if (state.queueOpen) {
    if (state.drawerTab === "queue" && (prevId !== state.playback.current?.id || prevQLen !== state.playback.queue.length)) {
      renderQueue();
    } else if (state.drawerTab === "lyrics" && prevId !== state.playback.current?.id) {
      loadLyricsForCurrent();
    }
  }
  if (prevId !== state.playback.current?.id) {
    await refreshLibrary();
  }
  schedulePoll();
}

function schedulePoll() {
  if (pollTimer) clearTimeout(pollTimer);
  if (state.playback.is_playing || state.playback.is_buffering) {
    pollTimer = setTimeout(syncPlayback, 300);
  } else {
    pollTimer = null;
  }
}

async function refreshLibrary() {
  state.library = await invoke("get_library");
  const cssEl = document.getElementById("user-custom-css");
  if (cssEl) cssEl.textContent = state.library.custom_css || "";
  renderNav();
}

async function refreshHome(forceFresh = false) {
  if (forceFresh) {
    state.homeRefreshNonce += 1;
    state.activeMood = null;
    if (state.page !== "home") {
      state.page = "home";
      renderNav();
    }
  }
  state.loadingHome = true;
  const hdrIcon = document.getElementById("icon-header-refresh");
  if (hdrIcon) hdrIcon.classList.add("animate-spin", "text-[#FF0033]");
  if (state.page === "home") renderMain();
  try {
    const excludeIds = forceFresh ? state.recommendations.map((t) => t.id) : [];
    const feed = await invoke("fetch_home_feed", {
      excludeIds,
      refreshNonce: state.homeRefreshNonce,
    });
    state.ytHistory = feed.yt_history || [];
    state.recommendations = feed.recommendations || [];
    state.ytConnected = feed.yt_connected;
  } catch (e) {
    console.error(e);
  }
  state.loadingHome = false;
  if (hdrIcon) hdrIcon.classList.remove("animate-spin", "text-[#FF0033]");
  renderNav();
  if (state.page === "home") {
    renderMain();
    const recEl = document.getElementById("carousel-rec");
    if (recEl) recEl.scrollTo({ left: 0, behavior: "smooth" });
  }
}

async function playTrackJson(track) {
  // Instant optimistic UI (< 5ms!)
  showBanner("");
  state.playback.current = track;
  state.playback.is_buffering = true;
  state.playback.is_playing = false;
  state.playback.position_secs = 0;
  renderPlayerBar();
  invoke("play_track", { track }).catch((e) => showBanner(`⚠ ${e}`, true));
  schedulePoll();
  if (state.queueOpen && state.drawerTab === "lyrics") loadLyricsForCurrent();
}

async function playCollectionIndex(idx) {
  if (!currentCollectionTracks.length) return;
  showBanner("");
  state.playback.current = currentCollectionTracks[idx];
  state.playback.is_buffering = true;
  state.playback.is_playing = false;
  state.playback.position_secs = 0;
  renderPlayerBar();
  invoke("play_collection", { tracks: currentCollectionTracks, startIdx: idx }).catch((e) =>
    showBanner(`⚠ ${e}`, true)
  );
  schedulePoll();
}

async function selectQueueIndex(idx) {
  if (state.playback.queue?.[idx]) {
    state.playback.current = state.playback.queue[idx];
    state.playback.current_index = idx;
    state.playback.is_buffering = true;
    state.playback.is_playing = false;
    renderPlayerBar();
    renderQueue();
  }
  invoke("select_queue_index", { idx }).catch((e) => showBanner(`⚠ ${e}`, true));
  schedulePoll();
}

async function cmdToggleShuffle() {
  await invoke("toggle_shuffle");
  await syncPlayback();
  if (state.queueOpen && state.drawerTab === "queue") renderQueue();
}

async function cmdCycleRepeat() {
  await invoke("cycle_repeat");
  await syncPlayback();
}

function toggleSleepMenu() {
  document.getElementById("sleep-menu").classList.toggle("hidden");
}

async function setSleep(minutes, stopAfterTrack) {
  document.getElementById("sleep-menu").classList.add("hidden");
  await invoke("set_sleep_timer", { minutes, stopAfterTrack });
  await syncPlayback();
}

async function cmdToggleMute() {
  const cur = state.playback.volume ?? 0.85;
  if (cur > 0.01) {
    state.mutedPrevVol = cur;
    await invoke("set_volume", { volume: 0 });
  } else {
    await invoke("set_volume", { volume: state.mutedPrevVol || 0.85 });
  }
  await syncPlayback();
}

async function saveCustomCss() {
  const css = document.getElementById("custom-css-input").value;
  await invoke("save_custom_css", { css });
  await refreshLibrary();
  showBanner("✓ Özel CSS uygulandı ve kaydedildi.");
  setTimeout(() => showBanner(""), 2200);
}

async function cmdTogglePause() {
  await invoke("toggle_pause");
  await syncPlayback();
}

async function cmdNext() {
  await invoke("play_next");
  await syncPlayback();
}

async function cmdPrev() {
  await invoke("play_prev");
  await syncPlayback();
}

async function cmdLikeCurrent() {
  if (!state.playback.current) return;
  await toggleLikeJson(state.playback.current);
}

async function toggleLikeJson(track) {
  await invoke("toggle_like", { track });
  await refreshLibrary();
  renderPlayerBar();
  renderMain();
}

async function addToPlaylistSelect(sel, track) {
  const playlistId = sel.value;
  sel.value = "";
  if (!playlistId) return;
  await invoke("add_to_playlist", { playlistId, track });
  await refreshLibrary();
  showBanner(`"${track.title}" oynatma listesine eklendi.`);
  setTimeout(() => showBanner(""), 2200);
}

async function removeFromPlaylist(playlistId, trackId) {
  await invoke("remove_from_playlist", { playlistId, trackId });
  await refreshLibrary();
  renderMain();
}

async function deletePlaylist(playlistId) {
  await invoke("delete_playlist", { playlistId });
  await refreshLibrary();
  navigate("home");
}

async function selectMood(label, query) {
  if (state.activeMood === label) {
    clearMood();
    return;
  }
  state.activeMood = label;
  state.loadingMood = true;
  renderMain();
  try {
    state.moodTracks = await invoke("search_songs", { query });
  } catch (_) {
    state.moodTracks = [];
  }
  state.loadingMood = false;
  renderMain();
}

function clearMood() {
  state.activeMood = null;
  state.moodTracks = [];
  renderMain();
}

async function connectBrowser(browser) {
  state.authBusy = true;
  state.authErr = "";
  state.authMsg = `${browser} tarayıcısından oturum alınıyor...`;
  renderMain();
  try {
    await invoke("connect_yt_browser", { browser });
    state.ytConnected = true;
    state.authMsg = `✓ ${browser} ile YouTube Music hesabın bağlandı!`;
    await refreshLibrary();
    await refreshHome();
  } catch (e) {
    state.authErr = String(e);
    state.authMsg = "";
  }
  state.authBusy = false;
  renderNav();
  renderMain();
}

async function connectCookies() {
  const raw = document.getElementById("cookie-input").value.trim();
  if (!raw) return;
  state.authBusy = true;
  state.authErr = "";
  state.authMsg = "Çerez doğrulanıyor...";
  renderMain();
  try {
    await invoke("connect_yt_cookies", { raw });
    state.ytConnected = true;
    state.authMsg = "✓ YouTube Music hesabın bağlandı!";
    await refreshLibrary();
    await refreshHome();
  } catch (e) {
    state.authErr = String(e);
    state.authMsg = "";
  }
  state.authBusy = false;
  renderNav();
  renderMain();
}

async function loginWithGoogle() {
  state.authBusy = true;
  state.authErr = "";
  state.authMsg = "Google oturum açma penceresi açıldı — girişini tamamla...";
  renderMain();
  try {
    const ok = await invoke("open_google_login");
    if (ok) {
      state.ytConnected = true;
      state.authMsg = "✓ Google hesabın bağlandı! YouTube geçmişin ve önerilerin eşitlendi.";
      await refreshLibrary();
      await refreshHome();
    } else {
      state.authMsg = "";
    }
  } catch (e) {
    state.authErr = String(e);
    state.authMsg = "";
  }
  state.authBusy = false;
  renderNav();
  renderMain();
}

async function disconnectYt() {
  await invoke("disconnect_yt");
  state.ytConnected = false;
  state.ytHistory = [];
  state.authMsg = "Bağlantı kaldırıldı.";
  renderNav();
  renderMain();
}

async function saveAndCheckUpdate() {
  const repo = document.getElementById("repo-input").value.trim();
  state.updateStatus = { ...state.updateStatus, kind: "checking" };
  renderMain();
  try {
    const info = await invoke("check_update", { repo });
    await refreshLibrary();
    if (info) {
      state.updateStatus = { ...state.updateStatus, kind: "available", info };
    } else {
      state.updateStatus = { ...state.updateStatus, kind: "uptodate", info: null };
    }
  } catch (e) {
    state.updateStatus = { ...state.updateStatus, kind: "error", message: String(e) };
  }
  renderNav();
  renderMain();
}

async function installUpdate() {
  if (!state.updateStatus.info) return;
  state.updateStatus = { ...state.updateStatus, kind: "downloading" };
  renderMain();
  try {
    await invoke("install_update", { info: state.updateStatus.info });
  } catch (e) {
    state.updateStatus = { ...state.updateStatus, kind: "error", message: String(e) };
    renderMain();
  }
}

// --- EVENT LISTENERS ---

document.getElementById("btn-toggle-sidebar").addEventListener("click", () => {
  state.sidebarExpanded = !state.sidebarExpanded;
  const sb = document.getElementById("sidebar");
  sb.classList.toggle("w-[240px]", state.sidebarExpanded);
  sb.classList.toggle("w-[72px]", !state.sidebarExpanded);
  document.querySelectorAll(".sidebar-label").forEach((el) => {
    el.classList.toggle("hidden", !state.sidebarExpanded);
  });
});

let suggestTimer = null;
let suggestReqId = 0;
let suggestItems = [];
let suggestActiveIdx = -1;

function hideSuggestions() {
  const box = document.getElementById("search-suggestions");
  if (box) box.classList.add("hidden");
  suggestActiveIdx = -1;
}

function renderSuggestions() {
  const box = document.getElementById("search-suggestions");
  if (!box) return;
  if (!suggestItems.length) {
    box.classList.add("hidden");
    return;
  }
  box.innerHTML = suggestItems
    .map((text, idx) => {
      const active = idx === suggestActiveIdx;
      return `
        <button type="button" onmousedown="event.preventDefault(); selectSuggestion(${idx})" onmouseenter="suggestActiveIdx=${idx}; renderSuggestions()" class="w-full flex items-center gap-3 px-3.5 py-2.5 text-left text-[13.5px] transition cursor-pointer ${
          active ? "bg-white/[0.14] text-white" : "text-neutral-200 hover:bg-white/[0.08]"
        }">
          <svg class="w-4 h-4 text-neutral-400 shrink-0" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
            <circle cx="11" cy="11" r="7" /><path stroke-linecap="round" d="m20 20-3.5-3.5" />
          </svg>
          <span class="truncate">${esc(text)}</span>
        </button>
      `;
    })
    .join("");
  box.classList.remove("hidden");
}

function selectSuggestion(idx) {
  const text = suggestItems[idx];
  if (!text) return;
  const input = document.getElementById("search-input");
  if (input) input.value = text;
  hideSuggestions();
  executeSearch(text);
}

async function executeSearch(rawQuery) {
  const query = (rawQuery || "").trim();
  if (!query) return;
  hideSuggestions();
  state.searchQuery = query;
  state.searching = true;
  navigate("search");
  try {
    state.searchResults = await invoke("search_songs", { query });
  } catch (err) {
    showBanner(`⚠ Arama hatası: ${err}`, true);
  }
  state.searching = false;
  renderMain();
}

const searchInputEl = document.getElementById("search-input");

searchInputEl.addEventListener("input", () => {
  const q = searchInputEl.value.trim();
  if (suggestTimer) clearTimeout(suggestTimer);
  if (!q) {
    suggestItems = [];
    hideSuggestions();
    return;
  }
  const reqId = ++suggestReqId;
  suggestTimer = setTimeout(async () => {
    try {
      const list = await invoke("search_suggestions", { query: q });
      if (reqId !== suggestReqId) return;
      suggestItems = Array.isArray(list) ? list : [];
      suggestActiveIdx = -1;
      renderSuggestions();
    } catch (_) {
      if (reqId === suggestReqId) hideSuggestions();
    }
  }, 130);
});

searchInputEl.addEventListener("focus", () => {
  if (searchInputEl.value.trim() && suggestItems.length) {
    renderSuggestions();
  }
});

searchInputEl.addEventListener("keydown", async (e) => {
  const box = document.getElementById("search-suggestions");
  const isOpen = box && !box.classList.contains("hidden") && suggestItems.length > 0;

  if (e.key === "ArrowDown" && isOpen) {
    e.preventDefault();
    suggestActiveIdx = (suggestActiveIdx + 1) % suggestItems.length;
    renderSuggestions();
    return;
  }
  if (e.key === "ArrowUp" && isOpen) {
    e.preventDefault();
    suggestActiveIdx = suggestActiveIdx <= 0 ? suggestItems.length - 1 : suggestActiveIdx - 1;
    renderSuggestions();
    return;
  }
  if (e.key === "Escape") {
    hideSuggestions();
    return;
  }
  if (e.key === "Enter") {
    e.preventDefault();
    if (isOpen && suggestActiveIdx >= 0 && suggestItems[suggestActiveIdx]) {
      selectSuggestion(suggestActiveIdx);
    } else {
      executeSearch(searchInputEl.value);
    }
  }
});

document.addEventListener("click", (e) => {
  const wrap = document.getElementById("search-box-wrap");
  if (wrap && !wrap.contains(e.target)) {
    hideSuggestions();
  }
});

document.getElementById("new-playlist-input").addEventListener("keydown", (e) => {
  if (e.key === "Enter") createPlaylist();
});

document.getElementById("volume-slider").addEventListener("input", async (e) => {
  const v = parseFloat(e.target.value);
  await invoke("set_volume", { volume: v });
});

document.getElementById("seek-bar-wrap").addEventListener("click", async (e) => {
  const rect = e.currentTarget.getBoundingClientRect();
  const frac = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
  const dur = Math.max(1, state.playback.duration_secs || state.playback.current?.duration_secs || 1);
  await invoke("seek", { seconds: frac * dur });
  await syncPlayback();
});

document.addEventListener("keydown", async (e) => {
  const tag = document.activeElement?.tagName;
  const typing = tag === "INPUT" || tag === "TEXTAREA";
  if ((e.ctrlKey && e.key.toLowerCase() === "k") || (!typing && e.key === "/")) {
    e.preventDefault();
    document.getElementById("search-input").focus();
    return;
  }
  if (e.key === "F5" || (e.ctrlKey && e.key.toLowerCase() === "r")) {
    e.preventDefault();
    refreshHome(true);
    return;
  }
  if (typing) return;
  if (e.code === "Space") {
    e.preventDefault();
    cmdTogglePause();
  } else if (e.ctrlKey && e.key === "ArrowRight") {
    e.preventDefault();
    cmdNext();
  } else if (e.ctrlKey && e.key === "ArrowLeft") {
    e.preventDefault();
    cmdPrev();
  } else if (e.key === "ArrowRight") {
    e.preventDefault();
    await invoke("seek", { seconds: (state.playback.position_secs || 0) + 5 });
    await syncPlayback();
  } else if (e.key === "ArrowLeft") {
    e.preventDefault();
    await invoke("seek", { seconds: Math.max(0, (state.playback.position_secs || 0) - 5) });
    await syncPlayback();
  }
});

let introVisible = false;

function showIntro(markSeen = false) {
  const splash = document.getElementById("app-splash");
  const vid = document.getElementById("intro-video");
  if (!splash || !vid) return;

  introVisible = true;
  vid.src = "intro.mp4";
  vid.currentTime = 0;
  vid.muted = false;

  splash.classList.remove("hidden", "pointer-events-none", "scale-[1.02]");
  // Force frame for smooth fade-in
  requestAnimationFrame(() => {
    splash.classList.remove("opacity-0");
  });

  vid.onended = hideSplash;
  vid.onerror = hideSplash;
  vid.play().catch(() => {
    vid.muted = true;
    vid.play().catch(hideSplash);
  });

  if (markSeen) {
    state.library.intro_seen = true;
    invoke("mark_intro_seen").catch(() => {});
  }
}

function hideSplash() {
  const splash = document.getElementById("app-splash");
  if (!splash || !introVisible) return;
  introVisible = false;
  const vid = document.getElementById("intro-video");
  if (vid) {
    try {
      vid.pause();
    } catch (_) {}
  }
  splash.classList.add("opacity-0", "scale-[1.02]", "pointer-events-none");
  setTimeout(() => {
    if (introVisible) return;
    splash.classList.add("hidden");
    if (vid) {
      vid.removeAttribute("src");
      vid.load();
    }
  }, 520);
}

async function runAutoUpdateOnStartup() {
  const repo = (state.library.update_repo || DEFAULT_UPDATE_REPO).trim();
  if (!repo) return;

  const toast = document.getElementById("update-check-toast");
  const toastText = document.getElementById("update-check-toast-text");
  if (toast) toast.classList.remove("hidden");

  try {
    // Strict 5-second ceiling on startup check
    const info = await Promise.race([
      invoke("check_update", { repo }),
      new Promise((_, reject) => setTimeout(() => reject(new Error("timeout")), 5000)),
    ]);

    if (toast) toast.classList.add("hidden");

    if (info) {
      state.updateStatus = { ...state.updateStatus, kind: "downloading", info };
      renderNav();

      const overlay = document.getElementById("auto-update-overlay");
      const title = document.getElementById("auto-update-title");
      const sub = document.getElementById("auto-update-sub");
      if (title) title.textContent = `Yeni sürüm (${info.version}) indiriliyor...`;
      if (sub) sub.textContent = `${info.asset_name} otomatik kurulup uygulama yeniden başlatılacak.`;
      if (overlay) overlay.classList.remove("hidden");

      await invoke("install_update", { info });
    }
  } catch (e) {
    if (toastText) toastText.textContent = "Güncelleme kontrolü atlandı";
    setTimeout(() => {
      if (toast) toast.classList.add("hidden");
    }, 600);
  }
}

// --- BOOT ---
(async function init() {
  document.addEventListener("keydown", (e) => {
    if (introVisible && (e.key === "Escape" || e.key === "Enter")) {
      e.preventDefault();
      hideSplash();
    }
  });

  await refreshLibrary();
  await syncPlayback();

  // Only play intro automatically on the very first launch (and never if music is already playing)
  if (!state.library.intro_seen && !state.playback.is_playing) {
    showIntro(true);
  }

  renderNav();
  renderMain();
  refreshHome();
  runAutoUpdateOnStartup();
})();
