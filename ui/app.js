const invoke = window.__TAURI__.core.invoke;
const DEFAULT_UPDATE_REPO = "Rbg4445/Ytmnative";

const THEME_PRESETS = {
  "": { label: "YouTube Music Dark Glass (Default)", css: "" },
  oled: {
    label: "OLED Pure Black",
    css: "body, #sidebar, header, #player-bar, #queue-drawer { background: #000000 !important; }",
  },
  crimson: {
    label: "Crimson Neon Glass",
    css: "#sidebar, #player-bar { background: rgba(28, 6, 14, 0.78) !important; border-color: rgba(255, 0, 51, 0.25) !important; }",
  },
  cyberpunk: {
    label: "Midnight Cyberpunk",
    css: "#sidebar, #player-bar { background: rgba(12, 10, 30, 0.80) !important; border-color: rgba(99, 102, 241, 0.3) !important; } #seek-bar-fill { background: #6366F1 !important; }",
  },
  emerald: {
    label: "Emerald Aurora",
    css: "#sidebar, #player-bar { background: rgba(6, 24, 18, 0.80) !important; border-color: rgba(16, 185, 129, 0.28) !important; } #seek-bar-fill { background: #10B981 !important; }",
  },
  frost: {
    label: "Nordic Frost Glass",
    css: "#sidebar, #player-bar, header { background: rgba(16, 24, 38, 0.72) !important; backdrop-filter: blur(28px) !important; }",
  },
};

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
  artistId: null,
  artistNameHint: null,
  artistDetail: null,
  loadingArtist: false,
  nowPlayingArtist: null,
  nowPlayingArtistKey: null,
  navHistory: [{ page: "home", id: null, extra: null }],
  navIdx: 0,
  libFilter: "all",
  libSearch: "",
  sidebarExpanded: true,
  queueOpen: false,
  homeCategory: "all",
  drawerTab: "now",
  lyricsTrackId: null,
  lyricsData: null,
  loadingLyrics: false,
  activeLyricIdx: -1,
  miniPlayer: false,
  alwaysOnTop: false,
  obsLayout: "default",
  mutedPrevVol: 0.85,
  activeMood: null,
  moodTracks: [],
  loadingMood: false,
  searchQuery: "",
  searchArtists: [],
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
    followed_artists: [],
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
  updateStatus: { kind: "idle", version: "4.0.0", info: null, message: "" },
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

function isArtistFollowed(artistId, artistName) {
  const list = state.library.followed_artists || [];
  return list.some(
    (a) =>
      (artistId && a.id === artistId) ||
      (artistName && a.name.toLowerCase() === artistName.toLowerCase())
  );
}

function fmtCount(n) {
  if (!n) return "";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(".0", "")}M abone`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1).replace(".0", "")}B abone`;
  return `${n} abone`;
}

function showBanner(msg, isError = false) {
  const el = document.getElementById("status-banner");
  if (!msg) {
    el.classList.add("hidden");
    return;
  }
  el.textContent = msg;
  el.className = isError
    ? "mx-6 mt-3 px-3.5 py-2 rounded-lg text-xs bg-[#3a0d16] text-red-200 border border-[#FF0033]/40"
    : "mx-6 mt-3 px-3.5 py-2 rounded-lg text-xs bg-white/10 text-neutral-200";
  el.classList.remove("hidden");
}

function navigate(page, id = null, extra = null, fromHistory = false) {
  state.page = page;
  if (page === "playlist") {
    state.playlistId = id;
  } else if (page === "artist") {
    state.artistId = id;
    state.artistNameHint = extra;
  }
  if (!fromHistory) {
    state.navHistory = state.navHistory.slice(0, state.navIdx + 1);
    state.navHistory.push({ page, id, extra });
    state.navIdx = state.navHistory.length - 1;
  }
  renderNav();
  renderMain();
}

function navBack() {
  if (state.navIdx > 0) {
    state.navIdx -= 1;
    const entry = state.navHistory[state.navIdx];
    if (entry.page === "artist") {
      openArtist(entry.id, entry.extra, true);
    } else {
      navigate(entry.page, entry.id, entry.extra, true);
    }
  }
}

function navForward() {
  if (state.navIdx + 1 < state.navHistory.length) {
    state.navIdx += 1;
    const entry = state.navHistory[state.navIdx];
    if (entry.page === "artist") {
      openArtist(entry.id, entry.extra, true);
    } else {
      navigate(entry.page, entry.id, entry.extra, true);
    }
  }
}

async function openArtist(artistId, artistName = null, fromHistory = false) {
  const cleanId = (artistId || "").trim();
  const cleanName = (artistName || "").split(",")[0].trim();
  if (!cleanId && !cleanName) return;

  state.loadingArtist = true;
  state.artistDetail = {
    id: cleanId || cleanName,
    name: cleanName || cleanId,
    avatar_url: null,
    description: null,
    subscriber_count: null,
    tracks: [],
    similar_artists: [],
  };
  navigate("artist", cleanId, cleanName, fromHistory);

  try {
    const detail = await invoke("get_artist", {
      artistId: cleanId,
      artistName: cleanName || null,
    });
    state.artistDetail = detail;
  } catch (e) {
    showBanner(`⚠ Sanatçı yüklenemedi: ${e}`, true);
  }
  state.loadingArtist = false;
  if (state.page === "artist") {
    renderMain();
  }
}

function openCurrentTrackArtist() {
  const cur = state.playback.current;
  if (!cur) return;
  openArtist(cur.artist_id || "", cur.artist);
}

async function toggleFollowArtistObj(artist) {
  const followed = await invoke("toggle_follow_artist", { artist });
  await refreshLibrary();
  renderMain();
  renderNowPlayingPanel();
  showBanner(
    followed
      ? `✓ ${artist.name} kitaplığına eklendi.`
      : `${artist.name} takipten çıkarıldı.`
  );
  setTimeout(() => showBanner(""), 2000);
}

function setLibFilter(filter) {
  state.libFilter = filter;
  ["all", "playlists", "artists"].forEach((k) => {
    const btn = document.getElementById(`lib-chip-${k}`);
    if (btn) {
      btn.className =
        filter === k
          ? "px-3 py-1 rounded-full text-xs font-medium bg-white text-black transition cursor-pointer shrink-0"
          : "px-3 py-1 rounded-full text-xs font-medium bg-white/[0.08] hover:bg-white/[0.15] text-white transition cursor-pointer shrink-0";
    }
  });
  renderNav();
}

function setLibSearch(val) {
  state.libSearch = (val || "").trim().toLowerCase();
  renderNav();
}

function toggleQueue() {
  state.queueOpen = !state.queueOpen;
  const drawer = document.getElementById("queue-drawer");
  const btn = document.getElementById("btn-toggle-queue");
  if (state.queueOpen) {
    drawer.classList.remove("hidden");
    drawer.classList.add("flex");
    btn.classList.add("text-[#FF0033]");
    renderNowPlayingPanel();
    renderQueue();
  } else {
    drawer.classList.add("hidden");
    drawer.classList.remove("flex");
    btn.classList.remove("text-[#FF0033]");
  }
}

function openDrawerTab(tab) {
  if (!state.queueOpen) {
    toggleQueue();
  }
  switchDrawerTab(tab);
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

function getSidebarArtists() {
  const map = new Map();
  for (const a of state.library.followed_artists || []) {
    if (a.name) map.set(a.name.toLowerCase(), a);
  }
  const recentSource = [
    ...(state.library.recently_played || []),
    ...(state.library.liked_tracks || []),
    ...(state.ytHistory || []),
  ];
  for (const t of recentSource) {
    const primaryName = (t.artist || "").split(",")[0].trim();
    if (!primaryName || primaryName === "Bilinmeyen Sanatçı") continue;
    const key = primaryName.toLowerCase();
    if (!map.has(key)) {
      map.set(key, {
        id: t.artist_id || "",
        name: primaryName,
        avatar_url: t.thumbnail_url || null,
        subscriber_count: null,
      });
    }
    if (map.size >= 14) break;
  }
  return Array.from(map.values());
}

function renderNav() {
  document.querySelectorAll(".m3-nav-item").forEach((btn) => {
    const active = btn.dataset.nav === state.page;
    if (active) {
      btn.classList.add("active");
    } else {
      btn.classList.remove("active");
    }
  });

  document.querySelectorAll(".nav-top-pill").forEach((btn) => {
    const active = btn.dataset.nav === state.page;
    btn.className = active
      ? "nav-top-pill w-9 h-9 rounded-full bg-white text-black flex items-center justify-center transition cursor-pointer shrink-0 shadow"
      : "nav-top-pill w-9 h-9 rounded-full bg-[#1f1f24] hover:bg-white/20 border border-white/10 flex items-center justify-center text-white transition cursor-pointer shrink-0";
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
      "w-8 h-8 rounded-full bg-neutral-700 hover:ring-2 hover:ring-white/40 flex items-center justify-center text-xs font-bold text-white transition cursor-pointer";
    av.textContent = "👤";
    av.title = "YouTube Hesabı Bağla (Geçmiş & Öneriler)";
  }

  // Updater pill
  const upd = document.getElementById("btn-updater-pill");
  if (state.updateStatus.kind === "available") {
    upd.textContent = `⬆ ${state.updateStatus.info.version} ${t("updateReady")}`;
    upd.className =
      "sidebar-label mt-2 pt-2 border-t border-white/10 text-left px-2 py-1 text-[11.5px] font-medium text-[#FF0033] hover:text-white transition cursor-pointer truncate";
  } else {
    upd.textContent = `v${state.updateStatus.version} • ${t("updates")}`;
    upd.className =
      "sidebar-label mt-2 pt-2 border-t border-white/10 text-left px-2 py-1 text-[11.5px] text-neutral-400 hover:text-white transition cursor-pointer truncate";
  }

  // Left Bento Card: Rich Library Items (Liked Songs + Playlists + Artists)
  const listEl = document.getElementById("sidebar-playlists");
  const q = state.libSearch || "";
  const filter = state.libFilter || "all";
  let html = "";

  if (filter === "all" || filter === "playlists") {
    const likedTitle = t("likedMusic");
    if (!q || likedTitle.toLowerCase().includes(q)) {
      const likedActive = state.page === "liked";
      html += `
        <button onclick="navigate('liked')" class="w-full flex items-center gap-3 p-2 rounded-lg transition cursor-pointer ${
          likedActive ? "bg-white/[0.14]" : "hover:bg-white/[0.07]"
        }">
          <div class="w-11 h-11 rounded-md bg-gradient-to-br from-[#6366F1] via-[#8B5CF6] to-[#FF0033] flex items-center justify-center text-lg text-white shrink-0 shadow">♥</div>
          <div class="sidebar-label min-w-0 text-left flex-1">
            <div class="text-[13.5px] font-semibold truncate ${likedActive ? "text-[#FF0033]" : "text-white"}">${esc(likedTitle)}</div>
            <div class="text-[11.5px] text-neutral-400 truncate">📌 ${esc(t("playlist"))} • ${state.library.liked_tracks.length} şarkı</div>
          </div>
        </button>
      `;
    }

    for (const pl of state.library.playlists) {
      if (q && !pl.name.toLowerCase().includes(q)) continue;
      const active = state.page === "playlist" && state.playlistId === pl.id;
      const cover = pl.tracks[0]?.thumbnail_url;
      html += `
        <button onclick="navigate('playlist', '${esc(pl.id)}')" class="w-full flex items-center gap-3 p-2 rounded-lg transition cursor-pointer ${
          active ? "bg-white/[0.14]" : "hover:bg-white/[0.07]"
        }">
          <div class="w-11 h-11 rounded-md bg-[#222228] overflow-hidden flex items-center justify-center text-neutral-400 shrink-0">
            ${cover ? `<img src="${esc(cover)}" loading="lazy" alt="" class="w-full h-full object-cover" />` : "♫"}
          </div>
          <div class="sidebar-label min-w-0 text-left flex-1">
            <div class="text-[13.5px] font-semibold truncate ${active ? "text-[#FF0033]" : "text-white"}">${esc(pl.name)}</div>
            <div class="text-[11.5px] text-neutral-400 truncate">${esc(t("playlist"))} • ${pl.tracks.length} şarkı</div>
          </div>
        </button>
      `;
    }
  }

  if (filter === "all" || filter === "artists") {
    const artists = getSidebarArtists();
    for (const art of artists) {
      if (q && !art.name.toLowerCase().includes(q)) continue;
      const active =
        state.page === "artist" &&
        ((art.id && state.artistId === art.id) ||
          state.artistNameHint?.toLowerCase() === art.name.toLowerCase());
      html += `
        <button onclick="openArtist('${esc(art.id || "")}', '${esc(art.name)}')" class="w-full flex items-center gap-3 p-2 rounded-lg transition cursor-pointer ${
          active ? "bg-white/[0.14]" : "hover:bg-white/[0.07]"
        }">
          <div class="w-11 h-11 rounded-full bg-[#222228] overflow-hidden flex items-center justify-center text-sm font-bold text-neutral-300 shrink-0 border border-white/10">
            ${
              art.avatar_url
                ? `<img src="${esc(art.avatar_url)}" loading="lazy" alt="" class="w-full h-full artist-round" />`
                : esc(art.name.charAt(0).toUpperCase())
            }
          </div>
          <div class="sidebar-label min-w-0 text-left flex-1">
            <div class="text-[13.5px] font-semibold truncate ${active ? "text-[#FF0033]" : "text-white"}">${esc(art.name)}</div>
            <div class="text-[11.5px] text-neutral-400 truncate">Sanatçı</div>
          </div>
        </button>
      `;
    }
  }

  listEl.innerHTML = html;
}

function renderQuickAccessTiles() {
  const curId = state.playback.current?.id;
  const items = [];

  // 1) Liked Songs tile
  items.push({
    kind: "liked",
    title: t("likedMusic"),
    cover: null,
    gradient: true,
  });

  // 2) User playlists
  for (const pl of state.library.playlists.slice(0, 2)) {
    items.push({
      kind: "playlist",
      id: pl.id,
      title: pl.name,
      cover: pl.tracks[0]?.thumbnail_url || null,
    });
  }

  // 3) Followed artists or recent tracks to fill up to 8 tiles (2x4 grid like Spotify Home!)
  for (const art of (state.library.followed_artists || []).slice(0, 2)) {
    if (items.length >= 8) break;
    items.push({
      kind: "artist",
      id: art.id,
      title: art.name,
      cover: art.avatar_url,
    });
  }

  const recentPool = [
    ...(state.ytHistory || []),
    ...(state.library.recently_played || []),
    ...(state.recommendations || []),
  ];
  const seenIds = new Set();
  for (const tr of recentPool) {
    if (items.length >= 8) break;
    if (seenIds.has(tr.id)) continue;
    seenIds.add(tr.id);
    items.push({
      kind: "track",
      track: tr,
      title: tr.title,
      cover: tr.thumbnail_url,
    });
  }

  if (items.length <= 1) return "";

  return `
    <div class="quick-grid mb-7">
      ${items
        .map((it) => {
          if (it.kind === "liked") {
            return `
              <div onclick="navigate('liked')" class="quick-tile">
                <div class="w-[52px] h-[52px] bg-gradient-to-br from-[#6366F1] via-[#8B5CF6] to-[#FF0033] flex items-center justify-center text-lg text-white shrink-0">♥</div>
                <div class="text-[13px] font-bold text-white line-clamp-2 leading-tight">${esc(it.title)}</div>
                <div class="quick-tile-play">▶</div>
              </div>
            `;
          }
          if (it.kind === "playlist") {
            return `
              <div onclick="navigate('playlist', '${esc(it.id)}')" class="quick-tile">
                <div class="w-[52px] h-[52px] bg-[#222228] flex items-center justify-center text-neutral-400 shrink-0 overflow-hidden">
                  ${it.cover ? `<img src="${esc(it.cover)}" loading="lazy" alt="" class="w-full h-full object-cover" />` : "♫"}
                </div>
                <div class="text-[13px] font-bold text-white line-clamp-2 leading-tight">${esc(it.title)}</div>
                <div class="quick-tile-play">▶</div>
              </div>
            `;
          }
          if (it.kind === "artist") {
            return `
              <div onclick="openArtist('${esc(it.id || "")}', '${esc(it.title)}')" class="quick-tile">
                <div class="w-[52px] h-[52px] bg-[#222228] flex items-center justify-center text-neutral-300 shrink-0 overflow-hidden">
                  ${it.cover ? `<img src="${esc(it.cover)}" loading="lazy" alt="" class="w-full h-full object-cover" />` : "🎤"}
                </div>
                <div class="text-[13px] font-bold text-white line-clamp-2 leading-tight">${esc(it.title)}</div>
                <div class="quick-tile-play">▶</div>
              </div>
            `;
          }
          const safeJson = esc(JSON.stringify(it.track));
          const active = curId === it.track.id;
          return `
            <div onclick='playTrackJson(${safeJson})' onmouseenter="schedulePrefetch('${esc(it.track.id)}')" onmouseleave="cancelPrefetch()" class="quick-tile ${active ? "active" : ""}">
              <div class="w-[52px] h-[52px] bg-[#222228] shrink-0 overflow-hidden">
                ${it.cover ? `<img src="${esc(it.cover)}" loading="lazy" alt="" class="w-full h-full object-cover" />` : ""}
              </div>
              <div class="text-[13px] font-bold ${active ? "text-[#FF0033]" : "text-white"} line-clamp-2 leading-tight">${esc(it.title)}</div>
              <div class="quick-tile-play">▶</div>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderArtistCarousel(id, artists) {
  if (!artists || !artists.length) return "";
  return `
    <div id="${id}" class="flex gap-4 overflow-x-auto no-scrollbar scroll-smooth pb-2">
      ${artists
        .map((a) => {
          return `
            <div onclick="openArtist('${esc(a.id || "")}', '${esc(a.name)}')" class="group w-[156px] shrink-0 p-3 rounded-xl hover:bg-white/[0.06] transition cursor-pointer text-center">
              <div class="relative w-[130px] h-[130px] mx-auto rounded-full overflow-hidden bg-[#202024] shadow-md">
                ${
                  a.avatar_url
                    ? `<img src="${esc(a.avatar_url)}" loading="lazy" alt="" class="w-full h-full artist-round group-hover:scale-105 transition duration-300" />`
                    : `<div class="w-full h-full flex items-center justify-center text-3xl font-bold text-neutral-400">${esc(a.name.charAt(0))}</div>`
                }
              </div>
              <div class="mt-3 text-[14px] font-bold text-white truncate">${esc(a.name)}</div>
              <div class="mt-0.5 text-[12px] text-neutral-400 truncate">${esc(fmtCount(a.subscriber_count) || "Sanatçı")}</div>
            </div>
          `;
        })
        .join("")}
    </div>
  `;
}

function renderCardCarousel(id, tracks) {
  const curId = state.playback.current?.id;
  return `
    <div id="${id}" class="flex gap-4 overflow-x-auto no-scrollbar scroll-smooth pb-2">
      ${tracks
        .map((t) => {
          const active = curId === t.id;
          const safeJson = esc(JSON.stringify(t));
          const primaryArtist = (t.artist || "").split(",")[0].trim();
          return `
            <div class="group w-[176px] shrink-0 p-3 rounded-xl hover:bg-white/[0.06] transition">
              <div onclick='playTrackJson(${safeJson})' onmouseenter="schedulePrefetch('${esc(t.id)}')" onmouseleave="cancelPrefetch()" class="relative w-full aspect-square rounded-lg overflow-hidden bg-[#202024] shadow-md cursor-pointer">
                ${
                  t.thumbnail_url
                    ? `<img src="${esc(t.thumbnail_url)}" loading="lazy" alt="" class="w-full h-full object-cover group-hover:scale-[1.03] transition duration-300" />`
                    : ""
                }
                <div class="absolute inset-0 bg-black/20 group-hover:bg-black/40 transition flex items-end justify-end p-2.5">
                  <div class="w-10 h-10 rounded-full bg-[#FF0033] text-white flex items-center justify-center shadow-xl opacity-0 translate-y-2 group-hover:opacity-100 group-hover:translate-y-0 ${active ? "opacity-100 translate-y-0" : ""} transition-all">
                    <svg class="w-5 h-5 ml-0.5" fill="currentColor" viewBox="0 0 24 24">
                      <path d="M8 5v14l11-7z"/>
                    </svg>
                  </div>
                </div>
              </div>
              <div onclick='playTrackJson(${safeJson})' class="mt-2.5 text-[14px] font-semibold leading-snug line-clamp-1 cursor-pointer ${
                active ? "text-[#FF0033]" : "text-white"
              }">${esc(t.title)}</div>
              <div onclick="openArtist('${esc(t.artist_id || "")}', '${esc(primaryArtist)}')" class="mt-0.5 text-[12.5px] text-neutral-400 hover:text-white hover:underline line-clamp-1 cursor-pointer">${esc(t.artist)}</div>
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
          const primaryArtist = (t.artist || "").split(",")[0].trim();
          return `
            <div onmouseenter="schedulePrefetch('${esc(t.id)}')" onmouseleave="cancelPrefetch()" class="group flex items-center gap-3.5 py-2 px-2.5 rounded-lg hover:bg-white/[0.08] transition ${
              active ? "bg-white/[0.11]" : ""
            }">
              <span class="w-5 text-xs text-neutral-500 tabular-nums text-right shrink-0">${idx + 1}</span>
              <div onclick='${playCall}' class="relative w-10 h-10 rounded overflow-hidden bg-[#181818] shrink-0 cursor-pointer">
                ${
                  t.thumbnail_url
                    ? `<img src="${esc(t.thumbnail_url)}" loading="lazy" alt="" class="w-full h-full object-cover" />`
                    : ""
                }
                <div class="absolute inset-0 bg-black/45 ${
                  active ? "opacity-100" : "opacity-0 group-hover:opacity-100"
                } flex items-center justify-center transition">
                  <svg class="w-5 h-5 ${active ? "text-[#FF0033]" : "text-white"}" fill="currentColor" viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>
                </div>
              </div>
              <div class="flex-1 min-w-0">
                <div onclick='${playCall}' class="text-[14px] font-medium truncate cursor-pointer ${
                  active ? "text-[#FF0033]" : "text-white"
                }">${esc(t.title)}</div>
                <div onclick="openArtist('${esc(t.artist_id || "")}', '${esc(primaryArtist)}')" class="text-[12.5px] text-neutral-400 hover:text-white hover:underline truncate cursor-pointer inline-block max-w-full">${esc(t.artist)}</div>
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

function setHomeCategory(cat) {
  state.homeCategory = cat;
  renderMain();
}

function renderMain() {
  const main = document.getElementById("main-content");

  if (state.page === "home") {
    const recent =
      state.ytHistory.length > 0 ? state.ytHistory : state.library.recently_played;
    const recTitle =
      state.ytHistory.length > 0
        ? t("ytRecsTitle")
        : state.library.recently_played.length > 0
        ? t("personalRecsTitle")
        : t("quickPicksTitle");

    const cat = state.homeCategory || "all";

    main.innerHTML = `
      <!-- TOP CLEAN CATEGORY PILLS (SPOTIFY STYLE) -->
      <div class="flex items-center gap-2 mb-6">
        <button onclick="setHomeCategory('all')" class="px-4 py-1.5 rounded-full text-xs font-semibold transition cursor-pointer ${
          cat === 'all' ? 'bg-white text-black' : 'bg-white/[0.08] hover:bg-white/[0.15] text-white'
        }">Tümü</button>
        <button onclick="setHomeCategory('music')" class="px-4 py-1.5 rounded-full text-xs font-semibold transition cursor-pointer ${
          cat === 'music' ? 'bg-white text-black' : 'bg-white/[0.08] hover:bg-white/[0.15] text-white'
        }">Müzik</button>
        <button onclick="setHomeCategory('playlists')" class="px-4 py-1.5 rounded-full text-xs font-semibold transition cursor-pointer ${
          cat === 'playlists' ? 'bg-white text-black' : 'bg-white/[0.08] hover:bg-white/[0.15] text-white'
        }">Listeler</button>
        <button onclick="setHomeCategory('artists')" class="px-4 py-1.5 rounded-full text-xs font-semibold transition cursor-pointer ${
          cat === 'artists' ? 'bg-white text-black' : 'bg-white/[0.08] hover:bg-white/[0.15] text-white'
        }">Sanatçılar</button>
      </div>

      ${
        cat === "playlists"
          ? `
        <section class="mb-10">
          <h2 class="text-xl font-bold mb-4 text-white">Çalma Listelerin</h2>
          <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
            <div onclick="navigate('liked')" class="p-3.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.08] transition cursor-pointer">
              <div class="w-full aspect-square rounded-lg bg-gradient-to-br from-[#6366F1] via-[#8B5CF6] to-[#FF0033] flex items-center justify-center text-3xl text-white shadow-lg mb-3">♥</div>
              <div class="font-bold text-sm text-white truncate">${esc(t("likedMusic"))}</div>
              <div class="text-xs text-neutral-400 mt-1">${state.library.liked_tracks.length} şarkı</div>
            </div>
            ${state.library.playlists.map(pl => `
              <div onclick="navigate('playlist', '${esc(pl.id)}')" class="p-3.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.08] transition cursor-pointer">
                <div class="w-full aspect-square rounded-lg bg-[#222228] overflow-hidden flex items-center justify-center text-neutral-400 shadow-lg mb-3">
                  ${pl.tracks[0]?.thumbnail_url ? `<img src="${esc(pl.tracks[0].thumbnail_url)}" class="w-full h-full object-cover" />` : "♫"}
                </div>
                <div class="font-bold text-sm text-white truncate">${esc(pl.name)}</div>
                <div class="text-xs text-neutral-400 mt-1">${pl.tracks.length} şarkı</div>
              </div>
            `).join("")}
          </div>
        </section>`
          : cat === "artists"
          ? `
        <section class="mb-10">
          <h2 class="text-xl font-bold mb-4 text-white">Takip Ettiğin ve Önerilen Sanatçılar</h2>
          <div class="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 gap-4">
            ${getSidebarArtists().map(a => `
              <div onclick="openArtist('${esc(a.id || "")}', '${esc(a.name)}')" class="p-3.5 rounded-xl bg-white/[0.04] hover:bg-white/[0.08] transition cursor-pointer text-center">
                <div class="w-28 h-28 mx-auto rounded-full overflow-hidden bg-[#222228] shadow-md mb-3">
                  ${a.avatar_url ? `<img src="${esc(a.avatar_url)}" class="w-full h-full object-cover artist-round" />` : `<div class="w-full h-full flex items-center justify-center text-2xl font-bold text-neutral-400">${esc(a.name.charAt(0))}</div>`}
                </div>
                <div class="font-bold text-sm text-white truncate">${esc(a.name)}</div>
                <div class="text-xs text-neutral-400 mt-0.5">${esc(fmtCount(a.subscriber_count) || "Sanatçı")}</div>
              </div>
            `).join("")}
          </div>
        </section>`
          : `
        <!-- SPOTIFY-STYLE 2x4 QUICK ACCESS GRID -->
        ${renderQuickAccessTiles()}

        <!-- FEATURED RECOMMENDATIONS CAROUSEL -->
        <section class="mb-10">
          <div class="flex items-center justify-between mb-4">
            <h2 class="text-xl font-bold text-white tracking-tight">${esc(recTitle)}</h2>
            <div class="flex items-center gap-1.5">
              <button onclick="scrollCarousel('carousel-rec', -420)" class="w-7 h-7 rounded-full bg-black/40 hover:bg-white/15 flex items-center justify-center text-neutral-300 hover:text-white cursor-pointer transition">‹</button>
              <button onclick="scrollCarousel('carousel-rec', 420)" class="w-7 h-7 rounded-full bg-black/40 hover:bg-white/15 flex items-center justify-center text-neutral-300 hover:text-white cursor-pointer transition">›</button>
            </div>
          </div>
          ${renderCardCarousel("carousel-rec", state.recommendations)}
        </section>

        <!-- RECENT TRACKS / LISTEN AGAIN -->
        ${
          recent.length > 0
            ? `
          <section class="mb-10">
            <div class="flex items-center justify-between mb-4">
              <h2 class="text-xl font-bold text-white tracking-tight">${esc(t("listenAgain"))}</h2>
              <div class="flex items-center gap-1.5">
                <button onclick="scrollCarousel('carousel-recent', -420)" class="w-7 h-7 rounded-full bg-black/40 hover:bg-white/15 flex items-center justify-center text-neutral-300 hover:text-white cursor-pointer transition">‹</button>
                <button onclick="scrollCarousel('carousel-recent', 420)" class="w-7 h-7 rounded-full bg-black/40 hover:bg-white/15 flex items-center justify-center text-neutral-300 hover:text-white cursor-pointer transition">›</button>
              </div>
            </div>
            ${renderCardCarousel("carousel-recent", recent)}
          </section>`
            : ""
        }

        <!-- FAVORITE ARTISTS ROW -->
        ${
          getSidebarArtists().length > 0
            ? `
          <section class="mb-8">
            <div class="flex items-center justify-between mb-4">
              <h2 class="text-xl font-bold text-white tracking-tight">Favori Sanatçıların</h2>
            </div>
            ${renderArtistCarousel("carousel-home-artists", getSidebarArtists())}
          </section>`
            : ""
        }
      `
      }
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
    currentCollectionTracks = state.searchResults;
    main.innerHTML = `
      <div class="mb-6">
        <div class="text-xs uppercase tracking-wider text-neutral-400 mb-1">ARAMA SONUÇLARI</div>
        <h1 class="text-2xl font-bold">"${esc(state.searchQuery)}" ${
          state.searching ? '<span class="text-sm font-normal text-[#FF0033] ml-2">aranıyor...</span>' : ""
        }</h1>
      </div>

      ${
        state.searchArtists && state.searchArtists.length > 0
          ? `
        <section class="mb-8">
          <div class="flex items-center justify-between mb-3">
            <h2 class="text-lg font-bold text-white">Sanatçılar</h2>
            <span class="text-xs text-neutral-400">Şarkılarını görmek için sanatçıya tıkla</span>
          </div>
          ${renderArtistCarousel("carousel-search-artists", state.searchArtists)}
        </section>`
          : ""
      }

      <section>
        <div class="flex items-center justify-between mb-3">
          <h2 class="text-lg font-bold text-white">Şarkılar</h2>
          ${
            state.searchResults.length > 0
              ? `<button onclick="playCollectionIndex(0)" class="px-4 py-1.5 rounded-full bg-white text-black text-xs font-semibold cursor-pointer">▶ Tümünü Çal</button>`
              : ""
          }
        </div>
        ${
          state.searchResults.length > 0
            ? renderTrackRows(state.searchResults)
            : state.searching
            ? `<p class="text-sm text-neutral-400">Aranıyor...</p>`
            : `<p class="text-sm text-neutral-400">Sonuç bulunamadı. Üstteki arama çubuğuna bir şarkı veya sanatçı yazıp Enter'a bas.</p>`
        }
      </section>
    `;
    return;
  }

  if (state.page === "artist") {
    const art = state.artistDetail || {
      id: state.artistId || "",
      name: state.artistNameHint || "Sanatçı",
      avatar_url: null,
      description: null,
      subscriber_count: null,
      tracks: [],
      similar_artists: [],
    };
    currentCollectionTracks = art.tracks || [];
    const followed = isArtistFollowed(art.id, art.name);
    const safeArtistSummary = esc(
      JSON.stringify({
        id: art.id || art.name,
        name: art.name,
        avatar_url: art.avatar_url || currentCollectionTracks[0]?.thumbnail_url || null,
        subscriber_count: art.subscriber_count || null,
      })
    );

    main.innerHTML = `
      <!-- ARTIST HERO BANNER -->
      <div class="relative rounded-2xl overflow-hidden bg-gradient-to-br from-[#2a0c18] via-[#15151c] to-[#0c0c10] border border-white/10 p-6 mb-7 flex flex-col sm:flex-row items-center sm:items-end gap-6 shadow-2xl">
        <div class="w-36 h-36 sm:w-44 sm:h-44 rounded-full overflow-hidden bg-[#222228] border-2 border-white/20 shadow-2xl shrink-0">
          ${
            art.avatar_url || currentCollectionTracks[0]?.thumbnail_url
              ? `<img src="${esc(art.avatar_url || currentCollectionTracks[0]?.thumbnail_url)}" alt="" class="w-full h-full artist-round" />`
              : `<div class="w-full h-full flex items-center justify-center text-5xl font-bold text-neutral-400">${esc(art.name.charAt(0))}</div>`
          }
        </div>
        <div class="flex-1 min-w-0 text-center sm:text-left">
          <div class="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full bg-[#FF0033]/20 border border-[#FF0033]/40 text-[11px] font-bold text-[#FF4D6D] mb-2">
            <span>✓ Doğrulanmış Sanatçı</span>
          </div>
          <h1 class="text-3xl sm:text-5xl font-extrabold tracking-tight text-white truncate">${esc(art.name)}</h1>
          ${
            art.subscriber_count
              ? `<div class="text-sm text-neutral-300 mt-1.5">${esc(fmtCount(art.subscriber_count))}</div>`
              : `<div class="text-sm text-neutral-400 mt-1.5">${currentCollectionTracks.length} popüler parça</div>`
          }
          ${
            art.description
              ? `<p class="text-xs text-neutral-400 mt-2 line-clamp-2 max-w-2xl">${esc(art.description)}</p>`
              : ""
          }
          <div class="flex flex-wrap items-center justify-center sm:justify-start gap-3 mt-5">
            ${
              currentCollectionTracks.length > 0
                ? `<button onclick="playCollectionIndex(0)" class="px-6 py-2.5 rounded-full bg-[#FF0033] hover:bg-[#e0002d] text-white text-sm font-bold shadow-lg transition cursor-pointer">▶ Oynat</button>`
                : ""
            }
            <button onclick='toggleFollowArtistObj(${safeArtistSummary})' class="px-5 py-2 rounded-full border ${
              followed
                ? "border-[#FF0033] bg-[#FF0033]/15 text-white"
                : "border-white/25 hover:border-white text-white"
            } text-xs font-semibold transition cursor-pointer">
              ${followed ? "✓ Takip Ediliyor" : "+ Takip Et"}
            </button>
          </div>
        </div>
      </div>

      <!-- ARTIST POPULAR TRACKS -->
      <section class="mb-8">
        <div class="flex items-center justify-between mb-3">
          <h2 class="text-xl font-bold text-white">Popüler Şarkılar ${
            state.loadingArtist ? '<span class="text-sm font-normal text-[#FF0033] ml-2">yükleniyor...</span>' : ""
          }</h2>
        </div>
        ${
          currentCollectionTracks.length > 0
            ? renderTrackRows(currentCollectionTracks, { collection: true })
            : state.loadingArtist
            ? `<p class="text-sm text-neutral-400">Sanatçının şarkıları yükleniyor...</p>`
            : `<p class="text-sm text-neutral-400">Bu sanatçı için parça bulunamadı.</p>`
        }
      </section>

      ${
        art.similar_artists && art.similar_artists.length > 0
          ? `
        <section class="mb-6">
          <h2 class="text-xl font-bold text-white mb-3">Hayranların Beğendiği Diğer Sanatçılar</h2>
          ${renderArtistCarousel("carousel-similar-artists", art.similar_artists)}
        </section>`
          : ""
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
      <h1 class="text-[28px] font-bold mb-2">${esc(t("accountTitle"))}</h1>
      <p class="text-sm text-neutral-400 mb-6">${esc(t("accountSub"))}</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-6 space-y-5">
        ${
          state.ytConnected
            ? `
            <div class="flex items-center justify-between">
              <div class="flex items-center gap-2.5">
                <span class="w-2.5 h-2.5 rounded-full bg-[#FF0033]"></span>
                <span class="font-semibold text-white">${esc(t("accountConnected"))}</span>
              </div>
              <div class="flex gap-2">
                <button onclick="refreshHome(true)" class="px-4 py-2 rounded-full bg-white text-black text-xs font-semibold cursor-pointer">${esc(t("syncHistory"))}</button>
                <button onclick="disconnectYt()" class="px-4 py-2 rounded-full border border-white/20 text-xs text-neutral-300 hover:text-white cursor-pointer">${esc(t("disconnect"))}</button>
              </div>
            </div>`
            : `
            <div class="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
              <div>
                <h3 class="text-base font-semibold text-white">${esc(t("googleSignInTitle"))}</h3>
                <p class="text-xs text-neutral-400 mt-1">${esc(t("googleSignInSub"))}</p>
              </div>
              <button ${state.authBusy ? "disabled" : ""} onclick="loginWithGoogle()" class="shrink-0 flex items-center justify-center gap-3 px-5 py-2.5 rounded-full bg-white hover:bg-neutral-100 disabled:opacity-50 text-[#1f1f1f] text-sm font-semibold shadow-lg transition cursor-pointer">
                <svg class="w-4 h-4 shrink-0" viewBox="0 0 24 24">
                  <path fill="#4285F4" d="M23.49 12.27c0-.79-.07-1.54-.2-2.27H12v4.51h6.47a5.54 5.54 0 0 1-2.4 3.63v3.01h3.88c2.27-2.09 3.54-5.17 3.54-8.88Z"/>
                  <path fill="#34A853" d="M12 24c3.24 0 5.95-1.08 7.94-2.91l-3.88-3.01c-1.08.72-2.45 1.16-4.06 1.16-3.12 0-5.77-2.11-6.72-4.96H1.29v3.11A12 12 0 0 0 12 24Z"/>
                  <path fill="#FBBC05" d="M5.28 14.28A7.22 7.22 0 0 1 4.9 12c0-.79.14-1.56.38-2.28V6.61H1.29A12 12 0 0 0 0 12c0 1.94.46 3.77 1.29 5.39l3.99-3.11Z"/>
                  <path fill="#EA4335" d="M12 4.76c1.76 0 3.34.61 4.59 1.8l3.44-3.44C17.95 1.19 15.24 0 12 0A12 12 0 0 0 1.29 6.61l3.99 3.11c.95-2.85 3.6-4.96 6.72-4.96Z"/>
                </svg>
                <span>${state.authBusy ? esc(t("waitingLogin")) : esc(t("googleSignInBtn"))}</span>
              </button>
            </div>`
        }
        ${state.authMsg ? `<div class="text-xs text-emerald-400">${esc(state.authMsg)}</div>` : ""}
        ${state.authErr ? `<div class="text-xs text-[#FF0033]">⚠ ${esc(state.authErr)}</div>` : ""}
      </div>

      <h2 class="text-xl font-bold mt-8 mb-3">${esc(t("appSettings"))}</h2>

      <!-- LANGUAGE SELECTOR -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between gap-4 mb-4">
        <div>
          <div class="text-sm font-semibold text-white">🌐 ${esc(t("languageTitle"))}</div>
          <div class="text-xs text-neutral-400 mt-0.5">${esc(t("languageSub"))}</div>
        </div>
        <select id="language-select" onchange="changeLanguage(this.value)" class="shrink-0 bg-[#0a0a0e] border border-white/20 hover:border-white/40 rounded-lg px-3.5 py-2 text-xs font-medium text-white cursor-pointer focus:outline-none">
          ${LANGUAGES.map(
            (l) => `<option value="${l.code}" ${currentLang === l.code ? "selected" : ""}>${esc(l.label)}</option>`
          ).join("")}
        </select>
      </div>

      <!-- THEME PRESET SELECTOR (ytmdesktop2 Themes) -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between gap-4 mb-4">
        <div>
          <div class="text-sm font-semibold text-white">🎨 Theme Preset</div>
          <div class="text-xs text-neutral-400 mt-0.5">Choose a built-in glassmorphic theme or combine with Custom CSS below.</div>
        </div>
        <select id="theme-preset-select" onchange="changeThemePreset(this.value)" class="shrink-0 bg-[#0a0a0e] border border-white/20 hover:border-white/40 rounded-lg px-3.5 py-2 text-xs font-medium text-white cursor-pointer focus:outline-none">
          ${Object.entries(THEME_PRESETS)
            .map(
              ([code, obj]) =>
                `<option value="${code}" ${(state.library.theme_preset || "") === code ? "selected" : ""}>${esc(obj.label)}</option>`
            )
            .join("")}
        </select>
      </div>

      <!-- DISCORD RICH PRESENCE (ytmdesktop2 feature) -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between gap-4 mb-4">
        <div>
          <div class="text-sm font-semibold text-white">🎮 Discord Rich Presence</div>
          <div class="text-xs text-neutral-400 mt-0.5">Show the currently playing song, artist, album art, and live progress bar on Discord.</div>
        </div>
        <input id="chk-discord-rpc" type="checkbox" ${state.library.discord_rpc_enabled !== false ? "checked" : ""} onchange="saveIntegrations()" class="w-4 h-4 accent-[#FF0033] cursor-pointer" />
      </div>

      <!-- OBS BROWSER SOURCE OVERLAY & STREAM DECK API (ytmdesktop2 feature) -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 space-y-3 mb-4">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-semibold text-white">🎥 OBS Browser Source Overlay &amp; Stream Deck API</div>
            <div class="text-xs text-neutral-400 mt-0.5">Local server on <code>http://127.0.0.1:13091</code> for OBS overlays and Stream Deck control.</div>
          </div>
          <input id="chk-obs-api" type="checkbox" ${state.library.obs_api_enabled !== false ? "checked" : ""} onchange="saveIntegrations()" class="w-4 h-4 accent-[#FF0033] cursor-pointer" />
        </div>
        <div class="flex flex-wrap items-center gap-2">
          <select id="obs-layout-select" onchange="state.obsLayout = this.value; renderMain()" class="bg-[#0a0a0e] border border-white/20 rounded-lg px-3 py-1.5 text-xs text-white cursor-pointer">
            <option value="default" ${state.obsLayout === "default" ? "selected" : ""}>Layout: Default Card</option>
            <option value="compact" ${state.obsLayout === "compact" ? "selected" : ""}>Layout: Compact</option>
            <option value="stack" ${state.obsLayout === "stack" ? "selected" : ""}>Layout: Vertical Stack</option>
            <option value="badge" ${state.obsLayout === "badge" ? "selected" : ""}>Layout: Pill Badge</option>
            <option value="text" ${state.obsLayout === "text" ? "selected" : ""}>Layout: Minimal Text</option>
          </select>
          <input id="obs-url-box" type="text" readonly value="http://127.0.0.1:13091/embed/now-playing?layout=${esc(state.obsLayout || "default")}" class="flex-1 min-w-[220px] bg-[#0a0a0a] border border-white/15 rounded-lg px-3 py-1.5 text-xs font-mono text-neutral-200" />
          <button onclick="copyObsUrl()" class="px-4 py-1.5 rounded-full bg-white text-black text-xs font-semibold cursor-pointer">Copy OBS URL</button>
        </div>
      </div>

      <!-- LAST.FM SCROBBLER (ytmdesktop2 feature) -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 space-y-3 mb-4">
        <div class="flex items-center justify-between">
          <div>
            <div class="text-sm font-semibold text-white">📻 Last.fm Scrobbler &amp; Now Playing</div>
            <div class="text-xs text-neutral-400 mt-0.5">Automatically updates Now Playing and scrobbles tracks after 50% playback.</div>
          </div>
          <input id="chk-lastfm" type="checkbox" ${state.library.lastfm_enabled ? "checked" : ""} onchange="saveIntegrations()" class="w-4 h-4 accent-[#FF0033] cursor-pointer" />
        </div>
        <div class="grid grid-cols-1 sm:grid-cols-3 gap-2">
          <input id="lastfm-key" type="text" placeholder="Last.fm API Key" value="${esc(state.library.lastfm_api_key || "")}" class="bg-[#0a0a0a] border border-white/15 rounded-lg px-3 py-1.5 text-xs text-white" />
          <input id="lastfm-secret" type="password" placeholder="Shared Secret" value="${esc(state.library.lastfm_secret || "")}" class="bg-[#0a0a0a] border border-white/15 rounded-lg px-3 py-1.5 text-xs text-white" />
          <input id="lastfm-sk" type="password" placeholder="Session Key (sk)" value="${esc(state.library.lastfm_session_key || "")}" class="bg-[#0a0a0a] border border-white/15 rounded-lg px-3 py-1.5 text-xs text-white" />
        </div>
        <button onclick="saveIntegrations()" class="px-4 py-1.5 rounded-full bg-white/15 hover:bg-white/25 text-xs font-semibold text-white cursor-pointer">Save Last.fm Settings</button>
      </div>

      <!-- SHARED LINKS & TAMPERMONKEY USERSCRIPT (ytmdesktop2 feature) -->
      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between gap-4 mb-4">
        <div>
          <div class="text-sm font-semibold text-white">🔗 Shared Links &amp; Browser Userscript</div>
          <div class="text-xs text-neutral-400 mt-0.5">Paste any <code>music.youtube.com</code>, <code>youtu.be</code>, or <code>ytmd://</code> URL into Search to play directly.</div>
        </div>
        <button onclick="downloadUserscript()" class="shrink-0 px-4 py-2 rounded-full bg-white/10 hover:bg-white/20 border border-white/15 text-xs font-semibold text-white cursor-pointer">⬇ Userscript (.user.js)</button>
      </div>

      <div class="max-w-2xl bg-[#141414]/80 backdrop-blur-lg border border-white/10 rounded-xl p-5 flex items-center justify-between mb-6">
        <div>
          <div class="text-sm font-semibold text-white">${esc(t("introTitle"))}</div>
          <div class="text-xs text-neutral-400 mt-0.5">${esc(t("introSub"))}</div>
        </div>
        <button onclick="showIntro(false)" class="shrink-0 px-4 py-2 rounded-full bg-white/10 hover:bg-[#FF0033] border border-white/15 hover:border-[#FF0033] text-xs font-semibold text-white transition cursor-pointer">${esc(t("showIntroBtn"))}</button>
      </div>

      <h2 class="text-xl font-bold mb-2">${esc(t("customCssTitle"))}</h2>
      <p class="text-xs text-neutral-400 mb-3">${esc(t("customCssSub"))}</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-5 space-y-3">
        <textarea id="custom-css-input" rows="4" placeholder="/* e.g. #player-bar { background: #141414 !important; } */" class="w-full font-mono bg-[#0a0a0a] border border-white/15 rounded-lg p-2.5 text-xs text-white focus:outline-none focus:border-white/40">${esc(state.library.custom_css || "")}</textarea>
        <button onclick="saveCustomCss()" class="px-5 py-2 rounded-full bg-white hover:bg-neutral-200 text-black text-xs font-semibold cursor-pointer">${esc(t("saveCssBtn"))}</button>
      </div>
    `;
    return;
  }

  if (state.page === "updates") {
    const repo = state.library.update_repo || DEFAULT_UPDATE_REPO;
    const u = state.updateStatus;
    main.innerHTML = `
      <h1 class="text-[28px] font-bold mb-2">${esc(t("updatesTitle"))}</h1>
      <p class="text-sm text-neutral-400 mb-6">${esc(t("updatesSub").replace("{v}", u.version))}</p>
      <div class="max-w-2xl bg-[#141414] border border-white/10 rounded-xl p-6 space-y-4">
        <div class="flex gap-2.5">
          <input id="repo-input" type="text" value="${esc(repo)}" placeholder="owner/repo" class="flex-1 bg-[#0a0a0a] border border-white/15 rounded-lg px-3.5 py-2 text-sm text-white focus:outline-none focus:border-white/40" />
          <button onclick="saveAndCheckUpdate()" class="px-5 py-2 rounded-full bg-white hover:bg-neutral-200 text-black text-xs font-semibold cursor-pointer">${esc(t("saveCheckBtn"))}</button>
        </div>
        ${
          u.kind === "checking"
            ? `<div class="text-xs text-neutral-300">⟳ ${esc(t("checkingReleases"))}</div>`
            : u.kind === "uptodate"
            ? `<div class="text-xs text-emerald-400">${esc(t("upToDate"))}</div>`
            : u.kind === "downloading"
            ? `<div class="text-xs text-[#FF0033]">${esc(t("downloadingUpdate"))}</div>`
            : u.kind === "error"
            ? `<div class="text-xs text-[#FF0033]">⚠ ${esc(u.message)}</div>`
            : u.kind === "available"
            ? `
              <div class="p-4 rounded-lg bg-[#211014] border border-[#FF0033]/40 space-y-2.5">
                <div class="text-sm font-semibold text-white">${esc(t("newVersionReady"))} ${esc(u.info.version)} (${esc(u.info.asset_name)})</div>
                ${u.info.release_notes ? `<pre class="text-xs text-neutral-300 whitespace-pre-wrap">${esc(u.info.release_notes)}</pre>` : ""}
                <button onclick="installUpdate()" class="px-5 py-2 rounded-full bg-[#FF0033] hover:bg-[#e0002d] text-white text-xs font-semibold cursor-pointer">${esc(t("installNowBtn"))}</button>
              </div>`
            : ""
        }
      </div>
    `;
  }
}

function renderQueue() {
  if (!state.queueOpen) return;
  const list = document.getElementById("queue-list");
  if (!list) return;
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
        <div onclick="selectQueueIndex(${i})" onmouseenter="schedulePrefetch('${esc(t.id)}')" onmouseleave="cancelPrefetch()" class="flex items-center gap-2.5 p-1.5 rounded-md cursor-pointer transition ${
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

async function ensureNowPlayingArtistLoaded() {
  const cur = state.playback.current;
  if (!cur) return;
  const primaryName = (cur.artist || "").split(",")[0].trim();
  const key = `${cur.artist_id || ""}::${primaryName.toLowerCase()}`;
  if (state.nowPlayingArtistKey === key) return;
  state.nowPlayingArtistKey = key;
  try {
    const info = await invoke("get_artist", {
      artistId: cur.artist_id || "",
      artistName: primaryName || null,
    });
    if (state.nowPlayingArtistKey === key) {
      state.nowPlayingArtist = info;
      renderNowPlayingPanel();
    }
  } catch (_) {}
}

function renderNowPlayingPanel() {
  const panel = document.getElementById("now-playing-panel");
  if (!panel) return;
  const cur = state.playback.current;
  if (!cur) {
    panel.innerHTML = `
      <div class="h-full flex flex-col items-center justify-center text-center py-12 px-4">
        <div class="w-16 h-16 rounded-2xl bg-white/[0.06] border border-white/10 flex items-center justify-center text-2xl text-neutral-400 mb-3">♫</div>
        <div class="text-sm font-semibold text-white">Henüz bir parça çalmıyor</div>
        <div class="text-xs text-neutral-400 mt-1">Bir şarkı başlattığında kapak görseli, şarkı sözleri ve sanatçı kartı burada görünür.</div>
      </div>
    `;
    return;
  }

  ensureNowPlayingArtistLoaded();
  const primaryName = (cur.artist || "").split(",")[0].trim();
  const liked = isLiked(cur.id);
  const safeTrack = esc(JSON.stringify(cur));
  const artInfo = state.nowPlayingArtist;
  const artId = cur.artist_id || artInfo?.id || "";
  const artAvatar = artInfo?.avatar_url || cur.thumbnail_url || "";
  const artSubs = artInfo?.subscriber_count ? fmtCount(artInfo.subscriber_count) : "Sanatçı";
  const artFollowed = isArtistFollowed(artId, primaryName);
  const safeArtistSummary = esc(
    JSON.stringify({
      id: artId || primaryName,
      name: primaryName,
      avatar_url: artAvatar || null,
      subscriber_count: artInfo?.subscriber_count || null,
    })
  );

  const q = state.playback.queue || [];
  const nextIdx = (state.playback.current_index ?? -1) + 1;
  const nextTrack = q[nextIdx] || null;

  panel.innerHTML = `
    <!-- 1. LARGE ALBUM ARTWORK CARD -->
    <div class="rounded-xl overflow-hidden bg-[#18181f] border border-white/10 shadow-2xl">
      ${
        cur.thumbnail_url
          ? `<img src="${esc(cur.thumbnail_url)}" alt="" class="w-full aspect-square object-cover" />`
          : `<div class="w-full aspect-square flex items-center justify-center text-4xl text-neutral-500">♫</div>`
      }
    </div>

    <!-- 2. TRACK TITLE + ARTIST + LIKE BUTTON -->
    <div class="flex items-start justify-between gap-2 pt-1">
      <div class="min-w-0">
        <div class="text-[17px] font-bold text-white leading-snug truncate">${esc(cur.title)}</div>
        <div onclick="openArtist('${esc(artId)}', '${esc(primaryName)}')" class="text-[13.5px] text-neutral-400 hover:text-white hover:underline truncate cursor-pointer mt-0.5">${esc(cur.artist)}</div>
      </div>
      <button onclick='toggleLikeJson(${safeTrack})' title="Beğen" class="p-1.5 rounded-full hover:bg-white/10 transition cursor-pointer shrink-0 ${
        liked ? "text-[#FF0033]" : "text-neutral-400 hover:text-white"
      }">
        <svg class="w-5 h-5" fill="${liked ? "currentColor" : "none"}" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78Z"/></svg>
      </button>
    </div>

    <!-- 3. LIVE LYRICS ACCORDION CARD (Spotify Right-Panel Red Card!) -->
    <div onclick="switchDrawerTab('lyrics')" class="rounded-xl bg-gradient-to-br from-[#6b091e] to-[#360611] border border-[#FF0033]/30 p-3.5 cursor-pointer hover:border-[#FF0033]/60 transition shadow-lg">
      <div class="flex items-center justify-between mb-1.5">
        <span class="text-xs font-bold uppercase tracking-wider text-white">Şarkı Sözleri</span>
        <span class="text-[11px] font-semibold text-white/80 hover:text-white">Tamamını aç →</span>
      </div>
      <div id="now-playing-lyric-preview" class="text-[13px] font-semibold text-white/95 line-clamp-2 min-h-[2.2rem]">
        ${
          state.activeLyricIdx >= 0 && state.lyricsData?.synced?.[state.activeLyricIdx]
            ? esc(state.lyricsData.synced[state.activeLyricIdx].text)
            : state.lyricsData?.body
            ? esc(state.lyricsData.body.split("\n").slice(0, 2).join(" • "))
            : "Senkronize şarkı sözlerini görmek için tıkla..."
        }
      </div>
    </div>

    <!-- 4. ABOUT THE ARTIST CARD (Spotify "Sanatçı hakkında" Card!) -->
    <div class="rounded-xl overflow-hidden bg-[#181820] border border-white/10 shadow-lg">
      <div onclick="openArtist('${esc(artId)}', '${esc(primaryName)}')" class="relative h-36 bg-[#22222a] overflow-hidden cursor-pointer group">
        ${
          artAvatar
            ? `<img src="${esc(artAvatar)}" alt="" class="w-full h-full object-cover group-hover:scale-105 transition duration-300" />`
            : ""
        }
        <div class="absolute inset-0 bg-gradient-to-t from-black/85 via-black/30 to-black/30"></div>
        <div class="absolute top-3 left-3 text-xs font-bold text-white drop-shadow">Sanatçı hakkında</div>
      </div>
      <div class="p-3.5 space-y-2">
        <div class="flex items-center justify-between gap-2">
          <div onclick="openArtist('${esc(artId)}', '${esc(primaryName)}')" class="min-w-0 cursor-pointer">
            <div class="text-[15px] font-bold text-white hover:underline truncate">${esc(primaryName)}</div>
            <div class="text-xs text-neutral-400 truncate">${esc(artSubs)}</div>
          </div>
          <button onclick='toggleFollowArtistObj(${safeArtistSummary})' class="px-3.5 py-1 rounded-full border ${
            artFollowed
              ? "border-[#FF0033] bg-[#FF0033]/15 text-white"
              : "border-white/25 hover:border-white text-white"
          } text-xs font-semibold transition cursor-pointer shrink-0">
            ${artFollowed ? "Takip Ediliyor" : "Takip Et"}
          </button>
        </div>
        ${
          artInfo?.description
            ? `<p class="text-xs text-neutral-400 line-clamp-3 leading-relaxed">${esc(artInfo.description)}</p>`
            : `<p class="text-xs text-neutral-400">${esc(primaryName)} sanatçısının popüler şarkılarını ve benzer sanatçıları görmek için karta tıkla.</p>`
        }
      </div>
    </div>

    <!-- 5. NEXT IN QUEUE PREVIEW CARD -->
    ${
      nextTrack
        ? `
      <div class="rounded-xl bg-[#181820] border border-white/10 p-3 space-y-2">
        <div class="flex items-center justify-between">
          <span class="text-xs font-bold text-white">Sıradaki</span>
          <button onclick="switchDrawerTab('queue')" class="text-[11px] font-semibold text-neutral-400 hover:text-white cursor-pointer">Sırayı aç</button>
        </div>
        <div onclick="selectQueueIndex(${nextIdx})" class="flex items-center gap-2.5 p-1.5 rounded-lg hover:bg-white/[0.07] transition cursor-pointer">
          <img src="${esc(nextTrack.thumbnail_url || "")}" alt="" class="w-10 h-10 rounded object-cover bg-neutral-800 shrink-0" />
          <div class="min-w-0 flex-1">
            <div class="text-xs font-semibold text-white truncate">${esc(nextTrack.title)}</div>
            <div class="text-[11px] text-neutral-400 truncate">${esc(nextTrack.artist)}</div>
          </div>
        </div>
      </div>`
        : ""
    }
  `;
}

function switchDrawerTab(tab) {
  state.drawerTab = tab;
  const nTab = document.getElementById("tab-drawer-now");
  const qTab = document.getElementById("tab-drawer-queue");
  const lTab = document.getElementById("tab-drawer-lyrics");
  const nPanel = document.getElementById("now-playing-panel");
  const qList = document.getElementById("queue-list");
  const lPanel = document.getElementById("lyrics-panel");

  const activeCls = "text-xs font-bold tracking-wider uppercase text-white border-b-2 border-[#FF0033] pb-1 cursor-pointer";
  const idleCls = "text-xs font-bold tracking-wider uppercase text-neutral-400 hover:text-white pb-1 cursor-pointer";

  if (nTab) nTab.className = tab === "now" ? activeCls : idleCls;
  if (lTab) lTab.className = tab === "lyrics" ? activeCls : idleCls;
  if (qTab) qTab.className = tab === "queue" ? activeCls : idleCls;

  if (nPanel) nPanel.classList.toggle("hidden", tab !== "now");
  if (lPanel) lPanel.classList.toggle("hidden", tab !== "lyrics");
  if (qList) qList.classList.toggle("hidden", tab !== "queue");

  if (tab === "now") {
    renderNowPlayingPanel();
    loadLyricsForCurrent();
  } else if (tab === "queue") {
    renderQueue();
  } else {
    loadLyricsForCurrent();
  }
}

async function loadLyricsForCurrent() {
  const cur = state.playback.current;
  const panel = document.getElementById("lyrics-panel");
  if (!cur) {
    if (panel) panel.innerHTML = `<p class="text-xs text-neutral-400">${esc(t("lyricsEmpty"))}</p>`;
    return;
  }
  if (state.lyricsTrackId === cur.id && !state.loadingLyrics) {
    renderLyricsPanel();
    return;
  }
  state.lyricsTrackId = cur.id;
  state.lyricsData = null;
  state.activeLyricIdx = -1;
  state.loadingLyrics = true;
  if (panel) panel.innerHTML = `<p class="text-xs text-neutral-400">${esc(t("loading"))}</p>`;
  try {
    state.lyricsData = await invoke("get_lyrics", {
      videoId: cur.id,
      title: cur.title,
      artist: cur.artist,
    });
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
  if (!panel) return;
  if (!state.lyricsData || (!state.lyricsData.body && !state.lyricsData.synced)) {
    panel.innerHTML = `<p class="text-xs text-neutral-400">${esc(t("lyricsNotFound"))}</p>`;
    return;
  }
  if (Array.isArray(state.lyricsData.synced) && state.lyricsData.synced.length > 0) {
    panel.innerHTML = `
      <div class="space-y-2.5 py-2">
        ${state.lyricsData.synced
          .map(
            (line, i) => `
          <div id="lyric-line-${i}" onclick="seekToLyric(${line.time_secs})" class="px-2.5 py-1.5 rounded-lg text-[14px] leading-snug transition-all duration-200 cursor-pointer text-neutral-400 hover:text-white hover:bg-white/[0.06]">
            ${esc(line.text || "♪")}
          </div>`
          )
          .join("")}
      </div>
      ${state.lyricsData.footer ? `<div class="mt-4 text-[11px] text-neutral-500">${esc(state.lyricsData.footer)}</div>` : ""}
    `;
    updateSyncedLyricsHighlight(state.playback.position_secs || 0);
    return;
  }
  panel.innerHTML = `
    <div class="text-[14px] leading-7 text-neutral-100">${esc(state.lyricsData.body)}</div>
    ${state.lyricsData.footer ? `<div class="mt-6 text-[11px] text-neutral-500">${esc(state.lyricsData.footer)}</div>` : ""}
  `;
}

async function seekToLyric(seconds) {
  await invoke("seek", { seconds });
  await syncPlayback();
}

function updateSyncedLyricsHighlight(posSecs) {
  const synced = state.lyricsData?.synced;
  if (!Array.isArray(synced) || !synced.length) {
    const miniLine = document.getElementById("mini-lyric-line");
    if (miniLine) miniLine.textContent = "";
    return;
  }
  let idx = -1;
  for (let i = 0; i < synced.length; i++) {
    if (posSecs + 0.15 >= synced[i].time_secs) {
      idx = i;
    } else {
      break;
    }
  }
  const miniLine = document.getElementById("mini-lyric-line");
  if (miniLine) {
    miniLine.textContent = idx >= 0 ? synced[idx].text : "";
  }
  const nowPreview = document.getElementById("now-playing-lyric-preview");
  if (nowPreview && idx >= 0 && synced[idx]) {
    nowPreview.textContent = synced[idx].text;
  }
  if (idx === state.activeLyricIdx) return;
  const prevEl = document.getElementById(`lyric-line-${state.activeLyricIdx}`);
  if (prevEl) {
    prevEl.className =
      "px-2.5 py-1.5 rounded-lg text-[14px] leading-snug transition-all duration-200 cursor-pointer text-neutral-400 hover:text-white hover:bg-white/[0.06]";
  }
  state.activeLyricIdx = idx;
  const curEl = document.getElementById(`lyric-line-${idx}`);
  if (curEl) {
    curEl.className =
      "px-2.5 py-1.5 rounded-lg text-[15px] font-bold leading-snug transition-all duration-200 cursor-pointer text-white bg-[#FF0033]/20 border-l-2 border-[#FF0033]";
    curEl.scrollIntoView({ behavior: "smooth", block: "center" });
  }
}

function renderPlayerBar() {
  const pb = state.playback;
  const dur = Math.max(1, pb.duration_secs || pb.current?.duration_secs || 1);
  const pos = Math.min(dur, pb.position_secs || 0);
  const pct = pb.current ? (pos / dur) * 100 : 0;

  document.getElementById("seek-bar-fill").style.width = `${pct}%`;
  document.getElementById("player-time").textContent = `${fmtSec(pos)} / ${fmtSec(dur)}`;
  const curTimeEl = document.getElementById("player-time-cur");
  const durTimeEl = document.getElementById("player-time-dur");
  if (curTimeEl) curTimeEl.textContent = pb.current ? fmtSec(pos) : "0:00";
  if (durTimeEl) durTimeEl.textContent = pb.current ? fmtSec(dur) : "0:00";
  const miniSeek = document.getElementById("mini-seek-fill");
  if (miniSeek) miniSeek.style.width = `${pct}%`;
  updateSyncedLyricsHighlight(pos);

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
    const miniCov = document.getElementById("mini-cover");
    if (miniCov) miniCov.src = pb.current.thumbnail_url || "";
    const miniTit = document.getElementById("mini-title");
    if (miniTit) miniTit.textContent = pb.current.title;
    const miniArt = document.getElementById("mini-artist");
    if (miniArt) miniArt.textContent = pb.current.artist;
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
    if (prevId !== state.playback.current?.id) {
      renderNowPlayingPanel();
      loadLyricsForCurrent();
      renderQueue();
    } else if (state.drawerTab === "queue" && prevQLen !== state.playback.queue.length) {
      renderQueue();
    } else if (state.drawerTab === "now" && prevQLen !== state.playback.queue.length) {
      renderNowPlayingPanel();
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
  const presetEl = document.getElementById("theme-preset-css");
  if (presetEl) {
    const preset = THEME_PRESETS[state.library.theme_preset || ""];
    presetEl.textContent = preset ? preset.css : "";
  }
  renderNav();
}

async function changeThemePreset(presetCode) {
  state.library.theme_preset = presetCode;
  const presetEl = document.getElementById("theme-preset-css");
  if (presetEl) {
    const preset = THEME_PRESETS[presetCode || ""];
    presetEl.textContent = preset ? preset.css : "";
  }
  await saveIntegrations();
}

async function saveIntegrations() {
  const discordEl = document.getElementById("chk-discord-rpc");
  const obsEl = document.getElementById("chk-obs-api");
  const lfmEl = document.getElementById("chk-lastfm");
  const lfmKeyEl = document.getElementById("lastfm-key");
  const lfmSecEl = document.getElementById("lastfm-secret");
  const lfmSkEl = document.getElementById("lastfm-sk");

  const discordRpcEnabled = discordEl ? discordEl.checked : state.library.discord_rpc_enabled !== false;
  const obsApiEnabled = obsEl ? obsEl.checked : state.library.obs_api_enabled !== false;
  const lastfmEnabled = lfmEl ? lfmEl.checked : !!state.library.lastfm_enabled;
  const lastfmApiKey = lfmKeyEl ? lfmKeyEl.value : state.library.lastfm_api_key || "";
  const lastfmSecret = lfmSecEl ? lfmSecEl.value : state.library.lastfm_secret || "";
  const lastfmSessionKey = lfmSkEl ? lfmSkEl.value : state.library.lastfm_session_key || "";
  const themePreset = state.library.theme_preset || "";

  await invoke("save_integrations", {
    discordRpcEnabled,
    obsApiEnabled,
    lastfmEnabled,
    lastfmApiKey,
    lastfmSecret,
    lastfmSessionKey,
    themePreset,
  });
  await refreshLibrary();
}

function copyObsUrl() {
  const box = document.getElementById("obs-url-box");
  if (box && navigator.clipboard) {
    navigator.clipboard.writeText(box.value);
    showBanner("✓ OBS Browser Source URL copied to clipboard!", false);
  }
}

function downloadUserscript() {
  const script = `// ==UserScript==
// @name         YTM Native — Open Links in Desktop App
// @namespace    https://github.com/Rbg4445/Ytmnative
// @version      4.0.0
// @description  Soft-opens actionable YouTube Music links via ytmd:// protocol
// @match        *://music.youtube.com/*
// @grant        GM_registerMenuCommand
// ==/UserScript==
(function() {
  'use strict';
  if (location.pathname === '/watch' && location.search.includes('v=')) {
    const ytmdUrl = location.href.replace(/^https?:\\/\\//, 'ytmd://');
    console.log('[YTM Native] Ready:', ytmdUrl);
  }
})();`;
  const blob = new Blob([script], { type: "application/javascript" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = "ytm-native-open-links.user.js";
  a.click();
}

async function toggleMiniPlayer() {
  state.miniPlayer = !state.miniPlayer;
  const overlay = document.getElementById("mini-player-overlay");
  if (overlay) overlay.classList.toggle("hidden", !state.miniPlayer);
  await invoke("set_mini_player", { enabled: state.miniPlayer });
  if (state.miniPlayer && state.playback.current) {
    loadLyricsForCurrent();
  }
}

async function toggleAlwaysOnTop() {
  state.alwaysOnTop = !state.alwaysOnTop;
  const btn = document.getElementById("btn-pin-top");
  if (btn) {
    btn.className = state.alwaysOnTop
      ? "w-8 h-8 rounded-full bg-[#FF0033] text-white flex items-center justify-center text-xs transition cursor-pointer"
      : "w-8 h-8 rounded-full bg-white/[0.08] hover:bg-white/[0.16] border border-white/[0.10] flex items-center justify-center text-xs text-neutral-300 hover:text-white transition cursor-pointer";
  }
  await invoke("set_always_on_top", { enabled: state.alwaysOnTop });
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
  renderNowPlayingPanel();
  invoke("play_track", { track }).catch((e) => showBanner(`⚠ ${e}`, true));
  schedulePoll();
  loadLyricsForCurrent();
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
  sb.classList.toggle("w-[280px]", state.sidebarExpanded);
  sb.classList.toggle("w-[76px]", !state.sidebarExpanded);
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
  state.searchArtists = [];
  state.searchResults = [];
  navigate("search");
  try {
    const bundle = await invoke("search_all", { query });
    state.searchArtists = bundle.artists || [];
    state.searchResults = bundle.tracks || [];
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
  if (e.ctrlKey && e.key.toLowerCase() === "m") {
    e.preventDefault();
    toggleMiniPlayer();
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

function changeLanguage(lang) {
  if (!I18N[lang]) lang = "en";
  currentLang = lang;
  localStorage.setItem("ytm_lang", lang);
  state.library.language = lang;
  invoke("set_language", { language: lang }).catch(() => {});
  applyStaticTranslations();
  renderNav();
  renderMain();
  renderQueue();
}

// --- BOOT ---
(async function init() {
  // On Linux (WebKitGTK), keep background opaque to prevent NVIDIA/compositor ghosting
  if (navigator.userAgent.includes("Linux")) {
    document.documentElement.classList.add("linux-opaque");
    const ambRoot = document.getElementById("ambient-backdrop-root");
    if (ambRoot) ambRoot.classList.add("bg-[#030303]");
  }

  document.addEventListener("keydown", (e) => {
    if (introVisible && (e.key === "Escape" || e.key === "Enter")) {
      e.preventDefault();
      hideSplash();
    }
  });

  await refreshLibrary();
  const savedLang = (state.library.language || "en").trim();
  currentLang = I18N[savedLang] ? savedLang : "en";
  localStorage.setItem("ytm_lang", currentLang);
  applyStaticTranslations();

  await syncPlayback();

  // Only play intro automatically on the very first launch (and never if music is already playing)
  if (!state.library.intro_seen && !state.playback.is_playing) {
    showIntro(true);
  }

  renderNav();
  renderMain();
  renderNowPlayingPanel();
  refreshHome();
  runAutoUpdateOnStartup();
})();
