import "./styles.css";
import { open, save } from "@tauri-apps/plugin-dialog";
import * as api from "./api";
import type {
  Genre,
  Entry,
  ExternalLink,
} from "./types";

// ============================================================================
// 状态管理
// ============================================================================

let genres: Genre[] = [];
let entries: api.EntrySummary[] = [];
// 列表数据与分页状态
let totalCount = 0;
let currentEntry: Entry | null = null;
let originalImageIds: string[] = [];
// 编辑表单脏标记（未保存修改保护）
let formDirty = false;
// 通用确认回调：删除条目 / 恢复数据库共用（null 时确认按钮走默认删除逻辑）
let confirmAction: (() => void) | null = null;
let selectedGenreIds: string[] = [];
let selectedRatings: string[] = ["S", "A", "B", "C"];
let selectedTags: string[] = [];
let selectedYear: number | null = null;
// 列表多选（批量爬图用）
let selectedEntryIds = new Set<string>();
// 批量爬图状态
let batchActive = false;
let batchItems: { id: string; name: string }[] = [];
let batchIndex = 0;
let batchSourceId = "";
let batchSourceName = "";
let batchCurrentItem: { id: string; name: string } | null = null;
let batchCurrentCandidates: api.CoverCandidate[] = [];
let batchResolve: (() => void) | null = null;
let currentSearchQuery: api.SearchQuery = {
  keyword: null,
  search_field: null,
  genre_ids: [],
  ratings: ["S", "A", "B", "C"],
  tag_filter: [],
  year: null,
  sort_by: "updated_at",
  sort_order: "desc",
  offset: 0,
  limit: 100,
};

// ============================================================================
// DOM 元素工具
// ============================================================================

const $ = <T extends HTMLElement>(id: string): T => document.getElementById(id) as T;

// ============================================================================
// 工具函数
// ============================================================================

function showToast(message: string, type: "success" | "error" = "success") {
  const toastEl = $<HTMLDivElement>("toast");
  toastEl.textContent = message;
  toastEl.className = `toast ${type}`;
  toastEl.classList.remove("hidden");
  toastEl.setAttribute("aria-live", "polite");
  toastEl.setAttribute("role", "status");
  setTimeout(() => {
    toastEl.classList.add("hidden");
  }, 3000);
}

// 打开弹窗前记录焦点（关闭时还原，无障碍）
let lastFocused: HTMLElement | null = null;

function openModal(id: string) {
  lastFocused = document.activeElement as HTMLElement | null;
  $(id).classList.remove("hidden");
  // 焦点移入弹窗内第一个可聚焦元素
  const focusable = $(id).querySelector<HTMLElement>(
    'input:not([type="hidden"]), select, textarea, button, [tabindex]:not([tabindex="-1"])'
  );
  focusable?.focus();
}

function closeModal(id: string) {
  // 编辑弹窗有未保存修改时，先弹确认（嵌套 modal-confirm），确认后才关闭
  if (id === "modal-entry" && formDirty) {
    $("confirm-title").textContent = "放弃未保存的修改？";
    $("btn-confirm-delete").textContent = "放弃修改";
    $("btn-confirm-delete").className = "warning-btn";
    $("confirm-message").textContent =
      "表单有未保存的修改，关闭后将丢失。确定放弃吗？";
    confirmAction = () => {
      formDirty = false;
      closeModal("modal-entry");
    };
    openModal("modal-confirm");
    return;
  }

  $(id).classList.add("hidden");
  // 焦点管理：有剩余弹窗则移入最上层（修复嵌套确认弹窗关闭后焦点丢失），否则还原
  const openModals = Array.from(document.querySelectorAll(".modal")).filter(
    (m) => !m.classList.contains("hidden")
  );
  if (openModals.length > 0) {
    const top = openModals[openModals.length - 1] as HTMLElement;
    top
      .querySelector<HTMLElement>(
        'input:not([type="hidden"]), select, textarea, button, [tabindex]:not([tabindex="-1"])'
      )
      ?.focus();
  } else {
    lastFocused?.focus?.();
  }
  // 批量爬图中关闭选择弹窗 → 终止批量
  if (id === "modal-cover-pick" && batchResolve) {
    batchActive = false;
    const r = batchResolve;
    batchResolve = null;
    r();
  }
  // 批量模式下关闭来源选择弹窗 → 取消批量（仅当尚未选中数据源；选源成功不触发）
  if (id === "modal-cover-source" && batchActive && batchSourceId === "") {
    batchActive = false;
    selectedEntryIds.clear();
    updateBatchButton();
  }
}

function getGenreIcon(name: string): string {
  const icons: Record<string, string> = {
    游戏: "🎮",
    音乐: "🎵",
    动漫: "🎬",
    小说: "📚",
    影视剧: "🎥",
  };
  return icons[name] || "📁";
}

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "";
  return new Date(dateStr).toLocaleDateString("zh-CN");
}

// ============================================================================
// 渲染函数
// ============================================================================

function renderGenres() {
  const genreFilterEl = $<HTMLDivElement>("genre-filter");
  genreFilterEl.innerHTML = genres
    .map(
      (g) => `
    <div class="filter-item">
      <input type="checkbox" id="genre-${g.id}" value="${g.id}" ${
        selectedGenreIds.includes(g.id) ? "checked" : ""
      } />
      <label for="genre-${g.id}" style="color:${getGenreColor(g.name)}">${getGenreIcon(g.name)} ${g.name}</label>
    </div>
  `
    )
    .join("");

  const entryGenreSelect = $<HTMLSelectElement>("entry-genre");
  entryGenreSelect.innerHTML = genres
    .map((g) => `<option value="${g.id}">${g.name}</option>`)
    .join("");

  genreFilterEl.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    cb.addEventListener("change", () => {
      const value = (cb as HTMLInputElement).value;
      if ((cb as HTMLInputElement).checked) {
        if (!selectedGenreIds.includes(value)) {
          selectedGenreIds.push(value);
        }
      } else {
        selectedGenreIds = selectedGenreIds.filter((id) => id !== value);
      }
      loadEntries();
    });
  });
}

/// 是否有活跃筛选条件（决定空态文案；类型为白名单勾选，默认不勾=无筛选）
function hasActiveFilter(): boolean {
  return (
    !!currentSearchQuery.keyword ||
    selectedGenreIds.length > 0 ||
    selectedRatings.length < 4 ||
    selectedTags.length > 0 ||
    selectedYear !== null
  );
}

/// 错误信息友好化：截断长度、隐藏本地路径，避免裸奔后端报错
function formatError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  let msg = raw.replace(/[A-Za-z]:\\[^\s:)]+/g, "[本地路径]");
  if (msg.length > 100) msg = msg.slice(0, 100) + "…";
  return msg;
}

/// 图片 base64 缓存（会话内，path → dataUrl）
const imageCache = new Map<string, string>();

async function cachedImageBase64(path: string): Promise<string | null> {
  if (imageCache.has(path)) return imageCache.get(path)!;
  try {
    const base64 = await api.getImageBase64(path);
    const ext = path.split(".").pop()?.toLowerCase() || "jpg";
    const dataUrl = `data:image/${ext};base64,${base64}`;
    imageCache.set(path, dataUrl);
    return dataUrl;
  } catch (err) {
    console.error("Failed to load image:", path, err);
    return null;
  }
}

/// HTML 转义（所有用户内容插值必须经过此函数，防破版与注入）
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/// 类型专属色（与 styles.css --genre-* token 同步）
const GENRE_COLORS: Record<string, string> = {
  游戏: "#da77f2",
  音乐: "#f06595", // 粉，避开 S 级红 #ff6b6b
  动漫: "#748ffc",
  小说: "#69db7c",
  影视剧: "#ffa94d",
};

function getGenreColor(name: string): string {
  return GENRE_COLORS[name] || "#868e96";
}

/// 渲染一批条目卡片（append 时只追加不重绘；图片异步逐张填充，不阻塞渲染）
function renderEntryCards(list: api.EntrySummary[], append = false) {
  const entryListEl = $<HTMLDivElement>("entry-list");
  const html = list
    .map(
      (e, idx) => `
    <div class="entry-card" data-id="${e.id}" tabindex="0" role="button" aria-label="查看《${escapeHtml(e.name)}》详情">
      <input type="checkbox" class="entry-select" data-id="${e.id}" aria-label="选择《${escapeHtml(e.name)}》" ${
        selectedEntryIds.has(e.id) ? "checked" : ""
      } />
      <div class="entry-card-placeholder" data-idx="${idx}" style="background:${getGenreColor(
        e.genre_name
      )}22;color:${getGenreColor(e.genre_name)}">${getGenreIcon(e.genre_name)}</div>
      <div class="entry-card-content">
        <div class="entry-card-header">
          <span class="entry-card-title">${escapeHtml(e.name)}</span>
          <span class="rating-badge ${e.rating}">${e.rating}</span>
          <span class="entry-card-genre" style="color:${getGenreColor(e.genre_name)}">${escapeHtml(e.genre_name)}</span>
        </div>
        <p class="entry-card-preview">${escapeHtml(e.review_preview)}</p>
        ${
          e.tags.length > 0
            ? `<div class="entry-card-tags">
          ${e.tags
            .slice(0, 3)
            .map((t: string) => `<span class="entry-tag">${escapeHtml(t)}</span>`)
            .join("")}
          ${e.tags.length > 3 ? `<span class="entry-tag">+${e.tags.length - 3}</span>` : ""}
        </div>`
            : ""
        }
      </div>
    </div>
  `
    )
    .join("");

  if (append) {
    entryListEl.insertAdjacentHTML("beforeend", html);
  } else {
    entryListEl.innerHTML = html;
  }

  // 只对本次渲染的卡片绑定事件
  const cards = Array.from(entryListEl.querySelectorAll(".entry-card"));
  const startIdx = append ? cards.length - list.length : 0;
  for (let i = startIdx; i < cards.length; i++) {
    const card = cards[i] as HTMLElement;
    card.addEventListener("click", () => {
      const id = card.getAttribute("data-id")!;
      showEntryDetail(id);
    });
    card.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter" || ev.key === " ") {
        ev.preventDefault();
        const id = card.getAttribute("data-id")!;
        showEntryDetail(id);
      }
    });
  }

  // 多选 checkbox：不触发卡片点击，维护选中集合
  const checkboxes = Array.from(entryListEl.querySelectorAll(".entry-select"));
  for (let i = startIdx; i < checkboxes.length; i++) {
    const cb = checkboxes[i] as HTMLInputElement;
    cb.addEventListener("click", (ev) => ev.stopPropagation());
    cb.addEventListener("change", () => {
      const id = cb.getAttribute("data-id")!;
      if (cb.checked) {
        selectedEntryIds.add(id);
      } else {
        selectedEntryIds.delete(id);
      }
      updateBatchButton();
    });
  }

  // 异步加载主图：逐张填充占位，不阻塞列表渲染（走缓存）
  list.forEach(async (e, i) => {
    if (!e.primary_image) return;
    const dataUrl = await cachedImageBase64(e.primary_image);
    if (!dataUrl) return;
    const card = cards[startIdx + i];
    const placeholder = card?.querySelector<HTMLDivElement>(
      `.entry-card-placeholder[data-idx="${i}"]`
    );
    if (placeholder) {
      placeholder.innerHTML = `<img class="entry-card-image" src="${dataUrl}" alt="${escapeHtml(e.name)}" />`;
    }
  });
}

async function renderEntries() {
  const entryListEl = $<HTMLDivElement>("entry-list");
  const entryCountEl = $<HTMLSpanElement>("entry-count");
  const emptyStateEl = $<HTMLDivElement>("empty-state");
  const loadMoreWrapEl = $<HTMLDivElement>("load-more-wrap");

  if (entries.length === 0) {
    entryListEl.innerHTML = "";
    entryListEl.classList.add("hidden");
    loadMoreWrapEl.classList.add("hidden");
    emptyStateEl.classList.remove("hidden");
    // 区分：筛选空态 vs 真·空库
    const filtered = hasActiveFilter();
    $("empty-all").classList.toggle("hidden", filtered);
    $("empty-filtered").classList.toggle("hidden", !filtered);
    entryCountEl.textContent = "共 0 条作品";
    return;
  }

  entryListEl.classList.remove("hidden");
  emptyStateEl.classList.add("hidden");
  updateListMeta();

  renderEntryCards(entries, false);
}

/// 更新列表计数与加载更多按钮（不重绘卡片）
function updateListMeta() {
  const entryCountEl = $<HTMLSpanElement>("entry-count");
  const loadMoreWrapEl = $<HTMLDivElement>("load-more-wrap");
  entryCountEl.textContent = `显示 ${entries.length} / 共 ${totalCount} 条作品`;
  loadMoreWrapEl.classList.toggle("hidden", entries.length >= totalCount);
}

async function renderDetailModal(entry: Entry) {
  currentEntry = entry;

  $("detail-title").textContent = entry.name;
  const genreEl = $("detail-genre");
  const genreName = entry.genre_id
    ? genres.find((g) => g.id === entry.genre_id)?.name || ""
    : "";
  genreEl.textContent = genreName;
  genreEl.style.color = getGenreColor(genreName);

  const ratingEl = $("detail-rating");
  ratingEl.className = `rating-badge ${entry.rating}`;
  ratingEl.textContent = entry.rating;

  $("detail-date").textContent = formatDate(entry.tasting_date);
  $("detail-creator").textContent = entry.creator || "";
  $("detail-review").textContent = entry.review;

  const imagePromises = entry.images.map(async (img) => {
    return cachedImageBase64(img.path);
  });

  const images = await Promise.all(imagePromises);

  const mainImage = $<HTMLDivElement>("detail-main-image");
  const primaryImage = entry.images.find((img) => img.is_primary) || entry.images[0];
  if (primaryImage) {
    const idx = entry.images.findIndex((img) => img.id === primaryImage.id);
    mainImage.innerHTML = images[idx] 
      ? `<img src="${images[idx]}" alt="${escapeHtml(entry.name)}" />`
      : `<div class="placeholder">${getGenreIcon($("detail-genre").textContent || "")}</div>`;
  } else {
    mainImage.innerHTML = `<div class="placeholder">${getGenreIcon($("detail-genre").textContent || "")}</div>`;
  }

  const thumbnails = $("detail-thumbnails");
  thumbnails.innerHTML = entry.images
    .map(
      (img, idx) => `
    <img class="thumbnail ${primaryImage?.id === img.id ? "active" : ""}"
         src="${images[idx] || ""}"
         data-path="${escapeHtml(img.path)}"
         tabindex="0" role="button"
         aria-label="查看第 ${idx + 1} 张图" />
  `
    )
    .join("");

  const tagsEl = $("detail-tags");
  tagsEl.innerHTML = entry.tags
    .map((t) => `<span class="detail-tag">${escapeHtml(t)}</span>`)
    .join("");

  const linksEl = $("detail-links");
  linksEl.innerHTML = entry.links
    .map(
      (l) => `
    <a class="detail-link" href="${escapeHtml(l.url)}" target="_blank">${escapeHtml(l.label || l.url)}</a>
  `
    )
    .join("");

  thumbnails.querySelectorAll(".thumbnail").forEach((thumb, idx) => {
    thumb.addEventListener("click", () => {
      mainImage.innerHTML = images[idx] 
        ? `<img src="${images[idx]}" alt="${escapeHtml(entry.name)}" />`
        : `<div class="placeholder">${getGenreIcon($("detail-genre").textContent || "")}</div>`;
      thumbnails.querySelectorAll(".thumbnail").forEach((t) => t.classList.remove("active"));
      thumb.classList.add("active");
    });
    thumb.addEventListener("keydown", (ev: Event) => {
      const kev = ev as KeyboardEvent;
      if (kev.key === "Enter" || kev.key === " ") {
        ev.preventDefault();
        thumb.dispatchEvent(new MouseEvent("click"));
      }
    });
  });
}

// ============================================================================
// 分享卡片
// ============================================================================

function drawWrappedText(
  ctx: CanvasRenderingContext2D,
  text: string,
  x: number,
  y: number,
  maxWidth: number,
  lineHeight: number,
  maxLines: number
): void {
  let line = "";
  let lines = 0;
  let cy = y;
  for (const ch of text) {
    if (ch === "\n" || ctx.measureText(line + ch).width > maxWidth) {
      ctx.fillText(line, x, cy);
      line = "";
      cy += lineHeight;
      lines++;
      if (lines >= maxLines) {
        ctx.fillText("…", x, cy);
        return;
      }
      if (ch === "\n") continue;
    }
    line += ch;
  }
  if (line) ctx.fillText(line, x, cy);
}

async function generateShareCard(entry: Entry): Promise<string> {
  const W = 640;
  const H = 880;
  const canvas = document.createElement("canvas");
  canvas.width = W;
  canvas.height = H;
  const ctx = canvas.getContext("2d")!;

  // 背景
  ctx.fillStyle = "#181825";
  ctx.fillRect(0, 0, W, H);

  // 封面（顶部全宽，底部渐变融入背景）
  let hasCover = false;
  const primary = entry.images.find((i) => i.is_primary) || entry.images[0];
  if (primary) {
    try {
      const b64 = await api.getImageBase64(primary.path);
      const ext = primary.path.split(".").pop()?.toLowerCase() || "jpg";
      const img = new Image();
      await new Promise<void>((resolve, reject) => {
        img.onload = () => resolve();
        img.onerror = () => reject(new Error("封面加载失败"));
        img.src = `data:image/${ext};base64,${b64}`;
      });
      const coverH = Math.min(400, H * 0.45);
      const scale = Math.max(W / img.width, coverH / img.height);
      const dw = img.width * scale;
      const dh = img.height * scale;
      ctx.drawImage(img, (W - dw) / 2, 0, dw, dh);
      const grad = ctx.createLinearGradient(0, coverH - 120, 0, coverH);
      grad.addColorStop(0, "rgba(24,24,37,0)");
      grad.addColorStop(1, "rgba(24,24,37,1)");
      ctx.fillStyle = grad;
      ctx.fillRect(0, coverH - 120, W, 120);
      hasCover = true;
    } catch {
      // 封面加载失败，走占位布局
    }
  }

  // 标题
  const titleY = hasCover ? 456 : 80;
  ctx.fillStyle = "#ffffff";
  ctx.font = "bold 34px 'Microsoft YaHei', sans-serif";
  drawWrappedText(ctx, entry.name, 32, titleY, W - 64, 44, 2);

  // 等级徽章（与主 UI rating-badge 一致的彩色圆角底 + 白字）
  const ratingColors: Record<string, string> = {
    S: "#ff6b6b",
    A: "#ffa94d",
    B: "#69db7c",
    C: "#15aabf", // 青，避开动漫靛 #748ffc
  };
  const ratingColor = ratingColors[entry.rating] || "#a6adc8";
  ctx.font = "bold 22px 'Microsoft YaHei', sans-serif";
  ctx.fillStyle = ratingColor;
  ctx.beginPath();
  ctx.roundRect(32, titleY + 24, 44, 34, 8);
  ctx.fill();
  ctx.fillStyle = "#ffffff";
  ctx.fillText(entry.rating, 42, titleY + 51);
  const genreName = genres.find((g) => g.id === entry.genre_id)?.name || "";
  const metaText = `${genreName}${entry.creator ? " · " + entry.creator : ""}${
    entry.tasting_date ? " · " + formatDate(entry.tasting_date) : ""
  }`.slice(0, 40); // 超长截断，避免溢出画布
  ctx.fillStyle = "#a6adc8";
  ctx.font = "16px 'Microsoft YaHei', sans-serif";
  ctx.fillText(metaText, 92, titleY + 54);

  // 评价（最多 8 行，避免压到标签区）
  ctx.fillStyle = "#cdd6f4";
  ctx.font = "16px 'Microsoft YaHei', sans-serif";
  drawWrappedText(ctx, entry.review, 32, titleY + 104, W - 64, 28, 8);

  // 标签（圆角胶囊）
  if (entry.tags.length > 0) {
    const tagY = H - 78;
    ctx.font = "14px 'Microsoft YaHei', sans-serif";
    let tx = 32;
    for (const tag of entry.tags.slice(0, 6)) {
      const tw = ctx.measureText(tag).width + 24;
      if (tx + tw > W - 32) break;
      ctx.fillStyle = "#313244";
      ctx.beginPath();
      ctx.roundRect(tx, tagY - 18, tw, 26, 13);
      ctx.fill();
      ctx.fillStyle = "#cdd6f4";
      ctx.fillText(tag, tx + 12, tagY);
      tx += tw + 8;
    }
  }

  // 水印
  ctx.fillStyle = "#6c7086";
  ctx.font = "13px 'Microsoft YaHei', sans-serif";
  ctx.fillText("文艺作品品鉴 · Preference Database", 32, H - 26);

  return canvas.toDataURL("image/png");
}

// ============================================================================
// 统计面板
// ============================================================================

function renderStatsBars(
  containerId: string,
  data: [string, number][],
  colorFn: (label: string) => string
) {
  const el = $<HTMLDivElement>(containerId);
  if (data.length === 0) {
    el.innerHTML = `<div class="stats-empty">暂无数据</div>`;
    return;
  }
  const max = Math.max(1, ...data.map((d) => d[1]));
  el.innerHTML = data
    .map(
      ([label, count]) => `
    <div class="stats-bar-row">
      <span class="stats-bar-label">${escapeHtml(label)}</span>
      <div class="stats-bar-track">
        <div class="stats-bar-fill" style="width:${((count / max) * 100).toFixed(1)}%;background:${colorFn(
          label
        )}"></div>
      </div>
      <span class="stats-bar-count">${count}</span>
    </div>`
    )
    .join("");
}

// ============================================================================
// 数据加载
// ============================================================================

async function loadGenres() {
  try {
    genres = await api.getGenres();
    renderGenres();
  } catch (err) {
    console.error("Failed to load genres:", err);
    showToast("加载类型失败", "error");
  }
}

function renderTagFilter(tags: string[]) {
  const el = $<HTMLDivElement>("tag-filter");
  if (tags.length === 0) {
    el.innerHTML = `<div class="filter-empty">暂无标签</div>`;
    return;
  }
  el.innerHTML = tags
    .map(
      (t, i) => `
    <div class="filter-item">
      <input type="checkbox" id="tag-chk-${i}" data-idx="${i}" ${
        selectedTags.includes(t) ? "checked" : ""
      } />
      <label for="tag-chk-${i}">${t}</label>
    </div>
  `
    )
    .join("");

  el.querySelectorAll('input[type="checkbox"]').forEach((cb) => {
    cb.addEventListener("change", () => {
      const idx = parseInt((cb as HTMLInputElement).getAttribute("data-idx")!, 10);
      const tag = tags[idx];
      if ((cb as HTMLInputElement).checked) {
        if (!selectedTags.includes(tag)) selectedTags.push(tag);
      } else {
        selectedTags = selectedTags.filter((x) => x !== tag);
      }
      loadEntries();
    });
  });
}

function renderYearFilter(years: number[]) {
  const el = $<HTMLSelectElement>("year-filter");
  el.innerHTML =
    `<option value="">全部年份</option>` +
    years
      .map(
        (y) => `<option value="${y}" ${selectedYear === y ? "selected" : ""}>${y}</option>`
      )
      .join("");
}

async function loadTagFilter() {
  try {
    renderTagFilter(await api.getTags());
  } catch (err) {
    console.error("Failed to load tags:", err);
  }
}

async function loadYearFilter() {
  try {
    renderYearFilter(await api.getTastingYears());
  } catch (err) {
    console.error("Failed to load years:", err);
  }
}

async function loadEntries(append = false) {
  const loadingEl = $<HTMLDivElement>("loading");
  const entryListEl = $<HTMLDivElement>("entry-list");
  const emptyStateEl = $<HTMLDivElement>("empty-state");

  if (!append) {
    loadingEl.classList.remove("hidden");
    entryListEl.classList.add("hidden");
    emptyStateEl.classList.add("hidden");
  }

  try {
    currentSearchQuery.genre_ids = selectedGenreIds;
    currentSearchQuery.ratings = selectedRatings;
    currentSearchQuery.tag_filter = selectedTags;
    currentSearchQuery.year = selectedYear;
    currentSearchQuery.offset = append ? entries.length : 0;

    const [newEntries, count] = await Promise.all([
      api.getEntries(currentSearchQuery),
      append ? Promise.resolve(totalCount) : api.getEntriesCount({ ...currentSearchQuery }),
    ]);

    if (append) {
      entries = [...entries, ...newEntries];
      renderEntryCards(newEntries, true);
      updateListMeta();
    } else {
      totalCount = count;
      entries = newEntries;
      // 筛选/搜索/排序变化时清空隐藏的选中项，避免批量删除误删不可见条目
      selectedEntryIds.clear();
      updateBatchButton();
      await renderEntries();
    }
  } catch (err) {
    console.error("Failed to load entries:", err);
    showToast("加载作品失败", "error");
    // 加载失败时恢复旧列表，避免内容区永久白屏
    if (entries.length > 0) {
      entryListEl.classList.remove("hidden");
      emptyStateEl.classList.add("hidden");
      loadingEl.classList.add("hidden");
    }
  } finally {
    loadingEl.classList.add("hidden");
  }
}

/// 清除全部筛选条件并刷新
function clearFilters() {
  $<HTMLInputElement>("search-input").value = "";
  $<HTMLSelectElement>("search-field").value = "all";
  currentSearchQuery.keyword = null;
  currentSearchQuery.search_field = null;

  selectedGenreIds = [];
  selectedRatings = ["S", "A", "B", "C"];
  selectedTags = [];
  selectedYear = null;

  $<HTMLSelectElement>("year-filter").value = "";
  document.querySelectorAll(".rating-checkbox").forEach((cb) => {
    (cb as HTMLInputElement).checked = true;
  });
  renderGenres();
  loadTagFilter();
  loadYearFilter();

  currentSearchQuery.sort_by = "updated_at";
  currentSearchQuery.sort_order = "desc";
  $<HTMLSelectElement>("sort-by").value = "updated_at";
  $<HTMLSelectElement>("sort-order").value = "desc";

  selectedEntryIds.clear();
  updateBatchButton();
  loadEntries();
}

// 详情加载进行中标志（防双击重复加载）
let detailLoading = false;

async function showEntryDetail(id: string) {
  if (detailLoading) return;
  detailLoading = true;
  try {
    // 先开弹窗显示加载态，避免等待期间无反馈
    const bodyEl = document.querySelector<HTMLElement>("#modal-detail .detail-content");
    $("detail-title").textContent = "加载中…";
    bodyEl?.classList.add("hidden");
    openModal("modal-detail");
    const entry = await api.getEntry(id);
    await renderDetailModal(entry);
    $("detail-title").textContent = entry.name;
    bodyEl?.classList.remove("hidden");
  } catch (err) {
    console.error("Failed to load entry:", err);
    closeModal("modal-detail");
    showToast("加载详情失败", "error");
  } finally {
    detailLoading = false;
  }
}

// ============================================================================
// 表单处理
// ============================================================================

function resetEntryForm() {
  const form = $<HTMLFormElement>("entry-form");
  form.reset();
  $<HTMLInputElement>("entry-id").value = "";
  $<HTMLTextAreaElement>("entry-review").value = "";
  $<HTMLDivElement>("links-container").innerHTML = "";
  $<HTMLDivElement>("images-container").innerHTML = "";
  originalImageIds = [];
  formDirty = false;
}

async function populateEntryForm(entry: Entry) {
  formDirty = false;
  $<HTMLInputElement>("entry-id").value = entry.id;
  $<HTMLInputElement>("entry-name").value = entry.name;
  $<HTMLSelectElement>("entry-genre").value = entry.genre_id;
  $<HTMLInputElement>("entry-creator").value = entry.creator || "";
  $<HTMLSelectElement>("entry-rating").value = entry.rating;
  $<HTMLTextAreaElement>("entry-review").value = entry.review;
  $<HTMLInputElement>("entry-date").value = entry.tasting_date || "";
  $<HTMLInputElement>("entry-tags").value = entry.tags.join(", ");

  const linksContainer = $<HTMLDivElement>("links-container");
  linksContainer.innerHTML = entry.links
    .map(
      (l) => `
    <div class="link-item">
      <input type="text" class="link-label" value="${escapeHtml(l.label)}" placeholder="标签" />
      <input type="url" class="link-url" value="${escapeHtml(l.url)}" placeholder="URL" />
      <button type="button" class="icon-btn remove-link">×</button>
    </div>
  `
    )
    .join("");

  originalImageIds = entry.images.map((img) => img.id);

  const imagePromises = entry.images.map(async (img) => {
    return cachedImageBase64(img.path);
  });

  const images = await Promise.all(imagePromises);

  const imagesContainer = $<HTMLDivElement>("images-container");
  imagesContainer.innerHTML = entry.images
    .map(
      (img, idx) => `
    <div class="image-item" data-id="${img.id}" data-path="${escapeHtml(img.path)}">
      <img src="${images[idx] || ""}" />
      <button type="button" class="remove-btn" data-id="${img.id}">×</button>
    </div>
  `
    )
    .join("");

  // 为已有图片的删除按钮添加事件
  imagesContainer.querySelectorAll(".remove-btn[data-id]").forEach((btn) => {
    btn.addEventListener("click", () => {
      const item = btn.closest(".image-item");
      if (item) {
        item.remove();
        formDirty = true; // 图片删除计入脏表单
      }
    });
  });
}

async function handleEntrySubmit(e: Event) {
  e.preventDefault();

  const submitBtn = $<HTMLButtonElement>("btn-save-entry");
  if (submitBtn.disabled) return; // 防双击重复提交
  submitBtn.disabled = true;

  const id = $<HTMLInputElement>("entry-id").value;
    const name = $<HTMLInputElement>("entry-name").value.trim();
    const genreId = $<HTMLSelectElement>("entry-genre").value;
    const creator = $<HTMLInputElement>("entry-creator").value.trim() || null;
    const rating = $<HTMLSelectElement>("entry-rating").value;
    const review = $<HTMLTextAreaElement>("entry-review").value;
    const tastingDate = $<HTMLInputElement>("entry-date").value || null;
    const tagsStr = $<HTMLInputElement>("entry-tags").value;
    const tags = tagsStr
      ? tagsStr.split(/[,，]/).map((t) => t.trim()).filter(Boolean)
      : [];

  const linksContainer = $<HTMLDivElement>("links-container");
  const linkItems = linksContainer.querySelectorAll(".link-item");
  const links: ExternalLink[] = [];
  linkItems.forEach((item) => {
    const label = (item.querySelector(".link-label") as HTMLInputElement).value.trim();
    const url = (item.querySelector(".link-url") as HTMLInputElement).value.trim();
    if (url) {
      links.push({ id: "", entry_id: id, label, url });
    }
  });

  // 收集图片路径（只收集新添加的，已有图片通过 data-id 标记）
  const imagesContainer = $<HTMLDivElement>("images-container");
  const imageItems = imagesContainer.querySelectorAll(".image-item");
  const imagePaths: string[] = [];
  const remainingImageIds: string[] = [];
  
  imageItems.forEach((item) => {
    const path = item.getAttribute("data-path");
    const imgId = item.getAttribute("data-id");
    
    if (imgId) {
      remainingImageIds.push(imgId);
    } else if (path) {
      imagePaths.push(path);
    }
  });

  // 仅在编辑模式下执行图片删除检查：比较 originalImageIds 与 remainingImageIds
  if (id && originalImageIds.length > 0) {
    const removedImageIds = originalImageIds.filter((imgId) => !remainingImageIds.includes(imgId));
    for (const imgId of removedImageIds) {
      try {
        await api.deleteEntryImage(imgId);
      } catch (err) {
        console.error("Failed to delete image:", imgId, err);
      }
    }
  }

  try {
    let savedId: string;
    if (id) {
      await api.updateEntry({ id, name, genre_id: genreId, creator, rating, review, tasting_date: tastingDate, links, tags });
      savedId = id;
      showToast("更新成功");
    } else {
      const newEntry = await api.createEntry({ name, genre_id: genreId, creator, rating, review, tasting_date: tastingDate, links, tags });
      savedId = newEntry.id;
      showToast("创建成功");
    }

    // 保存新添加的图片路径到数据库
    for (let i = 0; i < imagePaths.length; i++) {
      const isPrimary = imagePaths.length > 0 && i === 0;
      await api.addEntryImage(savedId, imagePaths[i], isPrimary);
    }

    formDirty = false;
    closeModal("modal-entry");
    resetEntryForm();
    loadEntries();
  } catch (err) {
    console.error("Failed to save entry:", err);
    showToast("保存失败: " + formatError(err), "error");
  } finally {
    submitBtn.disabled = false;
  }
}
// ============================================================================
// 事件绑定
// ============================================================================

function bindEvents() {
  // 主题切换
  const btnTheme = $("btn-theme");
  btnTheme.addEventListener("click", () => {
    const isDark = document.documentElement.getAttribute("data-theme") === "dark";
    document.documentElement.setAttribute("data-theme", isDark ? "light" : "dark");
    localStorage.setItem("prefdb-theme", isDark ? "light" : "dark");
    $("btn-theme").textContent = isDark ? "🌙" : "☀️";
  });

  // 新增按钮
  $("btn-new").addEventListener("click", () => {
    resetEntryForm();
    $("modal-title").textContent = "新增作品";
    openModal("modal-entry");
  });

  $("btn-empty-add").addEventListener("click", () => {
    resetEntryForm();
    $("modal-title").textContent = "新增作品";
    openModal("modal-entry");
  });

  // 编辑按钮
  $("btn-edit-entry").addEventListener("click", async () => {
    if (!currentEntry) return;
    await populateEntryForm(currentEntry);
    $("modal-title").textContent = "编辑作品";
    closeModal("modal-detail");
    openModal("modal-entry");
  });

  // 分享卡片
  $("btn-share-card").addEventListener("click", async () => {
    if (!currentEntry) return;
    try {
      showToast("正在生成分享卡片...");
      const dataUrl = await generateShareCard(currentEntry);
      const safeName = currentEntry.name.replace(/[\\/:*?"<>|]/g, "_");
      const savePath = await save({
        title: "保存分享卡片",
        defaultPath: `${safeName}.png`,
        filters: [{ name: "PNG 图片", extensions: ["png"] }],
      });
      if (!savePath) return;
      await api.saveBase64Image(dataUrl, savePath);
      showToast("分享卡片已保存");
    } catch (err) {
      showToast("生成失败: " + formatError(err), "error");
    }
  });

  // 删除按钮
  $("btn-delete-entry").addEventListener("click", () => {
    if (!currentEntry) return;
    confirmAction = null;
    $("confirm-title").textContent = "确认删除";
    $("btn-confirm-delete").textContent = "删除";
    $("btn-confirm-delete").className = "danger-btn";
    $("confirm-message").textContent = `确定要删除《${currentEntry.name}》吗？此操作不可撤销。`;
    openModal("modal-confirm");
  });

  $("btn-confirm-delete").addEventListener("click", async () => {
    if (confirmAction) {
      const action = confirmAction;
      confirmAction = null;
      closeModal("modal-confirm");
      await action();
      return;
    }
    if (!currentEntry) return;
    try {
      await api.deleteEntries([currentEntry.id]);
      showToast("删除成功");
      closeModal("modal-confirm");
      closeModal("modal-detail");
      loadEntries();
    } catch (err) {
      showToast("删除失败: " + formatError(err), "error");
    }
  });

  // 表单提交
  $("entry-form").addEventListener("submit", handleEntrySubmit);

  // 表单变更标记（脏表单保护）
  $("entry-form").addEventListener("input", () => {
    formDirty = true;
  });
  $("entry-form").addEventListener("change", () => {
    formDirty = true;
  });

  // 模态框关闭 - 使用 data-modal 属性
  document.querySelectorAll("[data-modal]").forEach((el) => {
    el.addEventListener("click", () => {
      const modalId = el.getAttribute("data-modal");
      if (modalId) {
        closeModal(modalId);
      }
    });
  });

  // 点击模态框背景关闭（统一走 closeModal，保证批量爬图等钩子生效）
  document.querySelectorAll(".modal").forEach((modal) => {
    modal.addEventListener("click", (e) => {
      if (e.target === modal) {
        closeModal(modal.id);
      }
    });
  });

  // 搜索
  $("search-btn").addEventListener("click", () => {
    const keyword = $<HTMLInputElement>("search-input").value.trim();
    const field = $<HTMLSelectElement>("search-field").value;
    currentSearchQuery.keyword = keyword || null;
    currentSearchQuery.search_field = field === "all" ? null : field;
    loadEntries();
  });

  // 加载更多
  $("btn-load-more").addEventListener("click", () => {
    loadEntries(true);
  });

  // 清除筛选（筛选空态）
  $("btn-clear-filters").addEventListener("click", () => {
    clearFilters();
  });

  // 添加第一个作品（空态）
  $("btn-empty-add").addEventListener("click", () => {
    $("modal-title").textContent = "新增作品";
    resetEntryForm();
    openModal("modal-entry");
  });

  // 搜索防抖（输入 300ms 后自动搜索）
  let searchDebounce: number | undefined;
  $("search-input").addEventListener("input", () => {
    window.clearTimeout(searchDebounce);
    searchDebounce = window.setTimeout(() => {
      const keyword = $<HTMLInputElement>("search-input").value.trim();
      const field = $<HTMLSelectElement>("search-field").value;
      currentSearchQuery.keyword = keyword || null;
      currentSearchQuery.search_field = field === "all" ? null : field;
      loadEntries();
    }, 300);
  });

  $("search-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      $("search-btn").dispatchEvent(new Event("click"));
    }
  });

  // 排序
  $("sort-by").addEventListener("change", () => {
    currentSearchQuery.sort_by = $<HTMLSelectElement>("sort-by").value;
    loadEntries();
  });

  $("sort-order").addEventListener("change", () => {
    currentSearchQuery.sort_order = $<HTMLSelectElement>("sort-order").value;
    loadEntries();
  });

  // 等级筛选
  document.querySelectorAll(".rating-checkbox").forEach((cb) => {
    cb.addEventListener("change", () => {
      selectedRatings = Array.from(document.querySelectorAll(".rating-checkbox:checked")).map(
        (c) => (c as HTMLInputElement).value
      );
      loadEntries();
    });
  });

  // 品鉴年份筛选
  $("year-filter").addEventListener("change", () => {
    const v = $<HTMLSelectElement>("year-filter").value;
    selectedYear = v ? parseInt(v, 10) : null;
    loadEntries();
  });

  // 视图切换
  function updateViewButtons(view: "card" | "list") {
    const cardsBtn = $<HTMLButtonElement>("view-cards");
    const listBtn = $<HTMLButtonElement>("view-list");
    cardsBtn.setAttribute("aria-pressed", String(view === "card"));
    listBtn.setAttribute("aria-pressed", String(view === "list"));
  }

  $("view-cards").addEventListener("click", () => {
    const entryListEl = $<HTMLDivElement>("entry-list");
    entryListEl.className = "entry-list card-view";
    $("view-cards").classList.add("active");
    $("view-list").classList.remove("active");
    updateViewButtons("card");
    localStorage.setItem("prefdb-view", "card");
  });

  $("view-list").addEventListener("click", () => {
    const entryListEl = $<HTMLDivElement>("entry-list");
    entryListEl.className = "entry-list list-view";
    $("view-list").classList.add("active");
    $("view-cards").classList.remove("active");
    updateViewButtons("list");
    localStorage.setItem("prefdb-view", "list");
  });

  // 添加链接
  $("btn-add-link").addEventListener("click", () => {
    const container = $<HTMLDivElement>("links-container");
    const div = document.createElement("div");
    div.className = "link-item";
    div.innerHTML = `
      <input type="text" class="link-label" placeholder="标签" />
      <input type="url" class="link-url" placeholder="URL" />
      <button type="button" class="icon-btn remove-link">×</button>
    `;
    container.appendChild(div);
    formDirty = true; // 链接变更计入脏表单

    div.querySelector(".remove-link")?.addEventListener("click", () => {
      div.remove();
      formDirty = true;
    });
  });

  // 添加图片（原生文件对话框选择，复制到 cover_image 目录）
  $("btn-add-image").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      title: "选择图片",
      directory: false,
      filters: [
        { name: "图片", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] },
      ],
    });
    if (!selected || typeof selected !== "string") return;

    const title = $<HTMLInputElement>("entry-name").value.trim() || "未命名";
    const creator = $<HTMLInputElement>("entry-creator").value.trim() || null;

    showToast("正在导入图片...");
    try {
      const newPath = await api.importLocalImage(selected, title, creator);
      await addImageToContainer(newPath);
      showToast("图片已导入");
    } catch (err) {
      console.error("Import image failed:", err);
      showToast("导入失败: " + formatError(err), "error");
    }
  });

  // 拖放图片到 images-container
  const imagesContainer = $<HTMLDivElement>("images-container");
  imagesContainer.addEventListener("dragover", (e) => {
    e.preventDefault();
    imagesContainer.classList.add("drag-over");
  });
  imagesContainer.addEventListener("dragleave", () => {
    imagesContainer.classList.remove("drag-over");
  });
  imagesContainer.addEventListener("drop", async (e) => {
    e.preventDefault();
    imagesContainer.classList.remove("drag-over");

    const files = e.dataTransfer?.files;
    if (!files || files.length === 0) return;

    const title = $<HTMLInputElement>("entry-name").value.trim() || "未命名";
    const creator = $<HTMLInputElement>("entry-creator").value.trim() || null;

    for (const file of Array.from(files)) {
      // 尝试获取文件路径
      const filePath = (file as any).path;
      if (!filePath) {
        showToast("无法获取文件路径，请使用手动添加", "error");
        continue;
      }

      try {
        showToast("正在导入图片...");
        const newPath = await api.importLocalImage(filePath, title, creator);
        await addImageToContainer(newPath);
        showToast("图片已导入");
      } catch (err) {
        console.error("Import image failed:", err);
        showToast("导入失败: " + formatError(err), "error");
      }
    }
  });

  // 清除选中（常驻入口）
  $("btn-clear-selection").addEventListener("click", () => {
    selectedEntryIds.clear();
    updateBatchButton();
    loadEntries();
  });

  // 批量爬图
  $("btn-batch-cover").addEventListener("click", () => {
    if (selectedEntryIds.size === 0) return;
    batchItems = entries
      .filter((e) => selectedEntryIds.has(e.id))
      .map((e) => ({ id: e.id, name: e.name }));
    batchActive = true;
    batchIndex = 0;
    showCoverSourceModal(true);
  });

  // 批量删除
  $("btn-batch-delete").addEventListener("click", () => {
    if (selectedEntryIds.size === 0) return;
    const n = selectedEntryIds.size;
    $("confirm-title").textContent = "批量删除";
    $("btn-confirm-delete").textContent = "删除";
    $("btn-confirm-delete").className = "danger-btn";
    $("confirm-message").textContent = `确定删除选中的 ${n} 条作品吗？关联的封面图片将一并删除，此操作不可撤销。`;
    confirmAction = async () => {
      try {
        await api.deleteEntries(Array.from(selectedEntryIds));
        const deleted = selectedEntryIds.size;
        selectedEntryIds.clear();
        updateBatchButton();
        showToast(`已删除 ${deleted} 条作品`);
        loadEntries();
      } catch (err) {
        showToast("删除失败: " + formatError(err), "error");
      }
    };
    openModal("modal-confirm");
  });

  // 统计面板（与当前筛选联动）
  $("btn-stats").addEventListener("click", async () => {
    try {
      const stats = await api.getStats({ ...currentSearchQuery });
      $("stats-total").textContent = String(stats.total);
      const hasFilter = hasActiveFilter();
      $("stats-scope").textContent = hasFilter ? "当前筛选范围内" : "全部作品";
      renderStatsBars("stats-rating", stats.rating_dist, (v) => {
        const colors: Record<string, string> = { S: "#ff6b6b", A: "#ffa94d", B: "#69db7c", C: "#15aabf" };
        return colors[v] || "#a6adc8";
      });
      renderStatsBars("stats-genre", stats.genre_dist, (v) => getGenreColor(v));
      renderStatsBars("stats-year", stats.year_dist, () => "#a6adc8");
      openModal("modal-stats");
    } catch (err) {
      showToast("统计加载失败: " + formatError(err), "error");
    }
  });

  // 批量快捷操作
  $("btn-cover-first").addEventListener("click", () => {
    if (batchCurrentCandidates.length === 0) {
      showToast("没有可用候选图", "error");
      return;
    }
    if (batchCurrentItem) {
      downloadAndAddCoverBatch(batchCurrentItem, batchCurrentCandidates[0]);
    }
  });

  $("btn-cover-skip").addEventListener("click", () => {
    if (batchResolve) {
      const r = batchResolve;
      batchResolve = null;
      r();
    }
  });

  $("btn-cover-skip-all").addEventListener("click", () => {
    batchActive = false;
    if (batchResolve) {
      const r = batchResolve;
      batchResolve = null;
      r();
    }
  });

  // 联网搜索封面
  $("btn-fetch-cover").addEventListener("click", () => {
    showCoverSourceModal(false);
  });

  // 数据源使用方式筛选
  $("cover-usage-filter").addEventListener("change", () => {
    renderCoverSourceList();
  });

  // 更换来源
  $("btn-cover-change-source").addEventListener("click", () => {
    closeModal("modal-cover-pick");
    showCoverSourceModal();
  });

  // 添加类型
  $("btn-add-genre").addEventListener("click", () => {
    openModal("modal-genre");
  });

  $("genre-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = $<HTMLInputElement>("genre-name").value.trim();
    if (!name) return;

    try {
      await api.createGenre(name);
      showToast("类型添加成功");
      closeModal("modal-genre");
      $<HTMLInputElement>("genre-name").value = "";
      loadGenres();
    } catch (err) {
      showToast("添加失败: " + formatError(err), "error");
    }
  });

  // 导出
  $("btn-export").addEventListener("click", () => {
    // 无勾选时禁用"选中的作品"选项
    const selectedRadio = document.querySelector(
      'input[name="export-scope"][value="selected"]'
    ) as HTMLInputElement;
    selectedRadio.disabled = selectedEntryIds.size === 0;
    const selectedLabel = selectedRadio.closest("label") as HTMLLabelElement;
    selectedLabel.style.opacity = selectedRadio.disabled ? "0.5" : "1";
    openModal("modal-export");
  });

  $("export-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const scope = (document.querySelector('input[name="export-scope"]:checked') as HTMLInputElement).value;
    const format = $<HTMLSelectElement>("export-format").value;
    const includeImages = $<HTMLInputElement>("export-images").checked;

    let ids: string[] | null = null;
    let filter: api.SearchQuery | null = null;
    if (scope === "selected") {
      if (selectedEntryIds.size === 0) {
        showToast("请先在列表中勾选要导出的作品", "error");
        return;
      }
      ids = Array.from(selectedEntryIds);
    } else if (scope === "filtered") {
      // 当前筛选条件原样传给后端，导出全部命中条目（不截断）
      filter = { ...currentSearchQuery };
    }

    try {
      const filePath = await api.exportEntries(scope, format, includeImages, ids, filter);
      showToast("导出成功: " + filePath);
      closeModal("modal-export");
    } catch (err) {
      showToast("导出失败: " + formatError(err), "error");
    }
  });

  // 备份
  $("btn-backup").addEventListener("click", async () => {
    try {
      const path = await api.backupDatabase();
      showToast(`备份成功: ${path}`);
    } catch (err) {
      showToast("备份失败: " + formatError(err), "error");
    }
  });

  // 恢复数据库（导入备份）
  $("btn-restore").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      title: "选择数据库备份文件",
      filters: [{ name: "SQLite 数据库", extensions: ["db"] }],
    });
    if (!selected || typeof selected !== "string") return;

    $("confirm-title").textContent = "恢复数据库";
    $("btn-confirm-delete").textContent = "覆盖恢复";
    $("btn-confirm-delete").className = "warning-btn";
    $("confirm-message").textContent =
      "将用所选数据库覆盖当前全部数据（当前数据库会先自动备份到 backups/）。确定继续吗？";
    confirmAction = async () => {
      try {
        showToast("正在导入数据库...");
        await api.importDatabase(selected);
        showToast("数据库导入成功");
        await loadGenres();
        await loadTagFilter();
        await loadYearFilter();
        await loadEntries();
      } catch (err) {
        showToast("导入失败: " + formatError(err), "error");
      }
    };
    openModal("modal-confirm");
  });

  // 导入 JSON/CSV
  $("btn-import").addEventListener("click", async () => {
    const selected = await open({
      multiple: false,
      title: "选择导入文件",
      filters: [{ name: "JSON / CSV", extensions: ["json", "csv"] }],
    });
    if (!selected || typeof selected !== "string") return;

    const ext = selected.split(".").pop()?.toLowerCase() || "json";
    const format = ext === "csv" ? "csv" : "json";

    try {
      showToast("正在导入...");
      const result = await api.importEntries(selected, format);
      showToast(`导入完成：成功 ${result.imported} 条，失败 ${result.failed} 条`);
      if (result.errors.length > 0) {
        console.warn("Import errors:", result.errors.slice(0, 5));
      }
      await loadGenres();
      await loadTagFilter();
      await loadYearFilter();
      await loadEntries();
    } catch (err) {
      showToast("导入失败: " + formatError(err), "error");
    }
  });
}

// ============================================================================
// 封面爬取
// ============================================================================

const USAGE_LABELS: Record<string, string> = {
  general: "通用",
  movie: "影视",
  book: "图书",
  music: "音乐",
  anime: "动漫",
  game: "游戏",
};

const SOURCE_DESCRIPTIONS: Record<string, string> = {
  bing_general: "Bing 图片搜索，适合没有特定数据源时使用",
  douban_movie: "豆瓣电影，电影/电视剧封面",
  bangumi_anime: "Bangumi 番组计划，动画番剧封面",
  anilist_anime: "AniList，动画番剧封面",
  douban_book: "豆瓣读书，图书封面",
  itunes_music: "iTunes 商店，专辑封面",
  igdb_game: "Steam 商店（IGDB 无 key 时替代）",
  steam_game: "Steam 商店，游戏封面",
};

let coverSources: api.CoverSource[] = [];

async function showCoverSourceModal(batch = false) {
  if (!batch) {
    const name = $<HTMLInputElement>("entry-name").value.trim();
    if (!name) {
      showToast("请先填写作品名称", "error");
      return;
    }
  }

  try {
    if (coverSources.length === 0) {
      coverSources = await api.getCoverSources();
    }
    renderCoverSourceList(batch);
    openModal("modal-cover-source");
  } catch (err) {
    console.error("Failed to load cover sources:", err);
    showToast("加载数据源失败: " + formatError(err), "error");
  }
}

function renderCoverSourceList(batch = false) {
  const usage = $<HTMLSelectElement>("cover-usage-filter").value;
  const filtered = usage === "all" ? coverSources : coverSources.filter((s) => s.usage === usage);

  const listEl = $<HTMLDivElement>("cover-source-list");
  if (filtered.length === 0) {
    listEl.innerHTML = `<div class="cover-empty">此分类下没有可用的数据源</div>`;
    return;
  }

  listEl.innerHTML = filtered
    .map(
      (s) => `
    <div class="source-item" data-id="${s.id}" tabindex="0" role="button" aria-label="选择数据源：${s.name}">
      <div class="source-item-header">
        <span class="source-item-name">${s.name}</span>
        <span class="source-item-usage">${USAGE_LABELS[s.usage] || s.usage}</span>
      </div>
      <div class="source-item-desc">${SOURCE_DESCRIPTIONS[s.id] || s.source_type}</div>
    </div>
  `
    )
    .join("");

  listEl.querySelectorAll(".source-item").forEach((item) => {
    item.addEventListener("click", () => {
      const id = item.getAttribute("data-id")!;
      const source = coverSources.find((s) => s.id === id);
      if (!source) return;
      if (batch) {
        // 先记录来源再关闭弹窗，避免 closeModal 的取消钩子误杀批量
        batchSourceId = id;
        batchSourceName = source.name;
      }
      closeModal("modal-cover-source");
      if (batch) {
        processBatchQueue();
      } else {
        fetchAndShowCandidates(id, source.name);
      }
    });
    item.addEventListener("keydown", (ev: Event) => {
      const kev = ev as KeyboardEvent;
      if (kev.key === "Enter" || kev.key === " ") {
        ev.preventDefault();
        item.dispatchEvent(new MouseEvent("click"));
      }
    });
  });
}

// ============================================================================
// 批量爬取
// ============================================================================

function updateBatchButton() {
  const btn = $<HTMLButtonElement>("btn-batch-cover");
  btn.disabled = selectedEntryIds.size === 0;
  btn.title = selectedEntryIds.size > 0 ? `批量爬图（${selectedEntryIds.size} 条）` : "批量爬图";
  const delBtn = $<HTMLButtonElement>("btn-batch-delete");
  delBtn.disabled = selectedEntryIds.size === 0;
  delBtn.title = selectedEntryIds.size > 0 ? `删除选中（${selectedEntryIds.size}）` : "批量删除";
  // 选中状态可见性
  const infoEl = $<HTMLSpanElement>("selection-info");
  const countEl = $<HTMLSpanElement>("selection-count");
  infoEl.classList.toggle("hidden", selectedEntryIds.size === 0);
  countEl.textContent = String(selectedEntryIds.size);
}

/// 依次处理批量队列中的条目；用户每完成一条的选择后继续下一条
async function processBatchQueue() {
  $("btn-cover-change-source").classList.add("hidden");
  $("btn-cover-first").classList.remove("hidden");
  $("btn-cover-skip").classList.remove("hidden");
  $("btn-cover-skip-all").classList.remove("hidden");

  while (batchIndex < batchItems.length && batchActive) {
    const item = batchItems[batchIndex];
    batchIndex++;
    batchCurrentItem = item;
    batchCurrentCandidates = [];

    const infoEl = $("cover-search-info");
    infoEl.textContent = `（${batchIndex}/${batchItems.length}）${item.name} · ${batchSourceName}`;
    const grid = $("cover-candidates");
    grid.innerHTML = `<div class="cover-empty">🔍 正在搜索《${item.name}》的封面...</div>`;
    openModal("modal-cover-pick");

    try {
      const candidates = await api.fetchCoverCandidates(item.name, null, batchSourceId);
      batchCurrentCandidates = candidates;
      renderCoverCandidates(candidates, batchSourceName);
    } catch (err) {
      console.error("Batch fetch failed:", item.name, err);
      grid.innerHTML = `<div class="cover-empty">搜索失败：${formatError(err)}</div>`;
    }

    // 等待用户操作（点图 / 使用第一张 / 跳过 / 全部跳过 / 关闭）
    await new Promise<void>((resolve) => {
      batchResolve = resolve;
    });
    batchResolve = null;
  }

  // 批量结束：区分正常完成与中途取消
  const aborted = batchIndex < batchItems.length;
  batchActive = false;
  batchCurrentItem = null;
  batchCurrentCandidates = [];
  batchSourceId = "";
  batchSourceName = "";
  $("btn-cover-change-source").classList.remove("hidden");
  $("btn-cover-first").classList.add("hidden");
  $("btn-cover-skip").classList.add("hidden");
  $("btn-cover-skip-all").classList.add("hidden");
  closeModal("modal-cover-pick");
  selectedEntryIds.clear();
  updateBatchButton();
  showToast(
    aborted
      ? `已取消批量爬图（完成 ${batchIndex}/${batchItems.length}）`
      : "批量爬图完成"
  );
  loadEntries();
}

/// 批量下载封面并关联到条目（首图设为主图）
async function downloadAndAddCoverBatch(
  item: { id: string; name: string },
  candidate: api.CoverCandidate
) {
  showToast(`正在下载《${item.name}》封面...`);
  try {
    const localPath = await api.downloadCover(candidate.url, item.name, null);
    const entry = await api.getEntry(item.id);
    const isPrimary = entry.images.length === 0;
    await api.addEntryImage(item.id, localPath, isPrimary);
    showToast(`《${item.name}》封面已添加`);
  } catch (err) {
    console.error("Batch download failed:", item.name, err);
    showToast(`《${item.name}》封面下载失败: ${formatError(err)}`, "error");
  }
  if (batchResolve) {
    const r = batchResolve;
    batchResolve = null;
    r();
  }
}

async function fetchAndShowCandidates(sourceId: string, sourceName: string) {
  const title = $<HTMLInputElement>("entry-name").value.trim();
  const creator = $<HTMLInputElement>("entry-creator").value.trim() || null;

  const infoEl = $("cover-search-info");
  infoEl.textContent = `来源：${sourceName} · 关键词：${title}${creator ? " " + creator : ""}`;

  const grid = $("cover-candidates");
  grid.innerHTML = `<div class="cover-empty">🔍 正在从 ${sourceName} 搜索封面...</div>`;
  openModal("modal-cover-pick");

  try {
    const candidates = await api.fetchCoverCandidates(title, creator, sourceId);
    renderCoverCandidates(candidates, sourceName);
  } catch (err) {
    console.error("Fetch cover candidates failed:", err);
    grid.innerHTML = `<div class="cover-empty">搜索失败：${formatError(err)}</div>`;
  }
}

function renderCoverCandidates(candidates: api.CoverCandidate[], sourceName: string) {
  const grid = $("cover-candidates");
  if (candidates.length === 0) {
    grid.innerHTML = `<div class="cover-empty">未找到匹配的封面，请尝试其他来源</div>`;
    return;
  }

  grid.innerHTML = candidates
    .map(
      (c, idx) => `
    <div class="cover-cell" data-idx="${idx}" tabindex="0" role="button" aria-label="${c.title ? "选择封面：" + c.title : "选择封面 第" + (idx + 1) + "张"}">
      <div class="cover-loading">加载中...</div>
      <div class="cover-source-tag">${sourceName}</div>
      ${c.title ? `<div class="cover-title">${c.title}</div>` : ""}
    </div>
  `
    )
    .join("");

  // 异步加载缩略图/原图
  candidates.forEach((c, idx) => {
    const cell = grid.querySelector(`.cover-cell[data-idx="${idx}"]`);
    if (!cell) return;
    const img = new Image();
    img.onload = () => {
      const loading = cell.querySelector(".cover-loading");
      if (loading) loading.remove();
      cell.insertBefore(img, cell.firstChild);
    };
    img.onerror = () => {
      const loading = cell.querySelector(".cover-loading");
      if (loading) loading.textContent = "加载失败";
    };
    img.src = c.thumbnail_url || c.url;
    img.alt = c.title || "cover";
  });

  // 绑定点击下载事件
  grid.querySelectorAll(".cover-cell").forEach((cell) => {
    cell.addEventListener("click", () => {
      const idx = parseInt(cell.getAttribute("data-idx")!, 10);
      const candidate = candidates[idx];
      if (!candidate) return;
      if (batchActive && batchCurrentItem) {
        downloadAndAddCoverBatch(batchCurrentItem, candidate);
      } else {
        downloadAndAddCover(candidate);
      }
    });
    cell.addEventListener("keydown", (ev: Event) => {
      const kev = ev as KeyboardEvent;
      if (kev.key === "Enter" || kev.key === " ") {
        ev.preventDefault();
        cell.dispatchEvent(new MouseEvent("click"));
      }
    });
  });
}

async function downloadAndAddCover(candidate: api.CoverCandidate) {
  const title = $<HTMLInputElement>("entry-name").value.trim();
  const creator = $<HTMLInputElement>("entry-creator").value.trim() || null;

  if (!title) {
    showToast("作品名称不能为空", "error");
    return;
  }

  showToast("正在下载封面...");
  try {
    const localPath = await api.downloadCover(candidate.url, title, creator);
    addImageToContainer(localPath);
    showToast("封面已下载并添加");
    closeModal("modal-cover-pick");
  } catch (err) {
    console.error("Download cover failed:", err);
    showToast("下载失败: " + formatError(err), "error");
  }
}

async function addImageToContainer(localPath: string) {
  const container = $<HTMLDivElement>("images-container");
  const div = document.createElement("div");
  div.className = "image-item";
  
  try {
    const dataUrl = await cachedImageBase64(localPath);
    if (!dataUrl) throw new Error("图片加载失败");
    div.innerHTML = `
      <img src="${dataUrl}" />
      <button type="button" class="remove-btn">×</button>
    `;
  } catch (err) {
    console.error("Failed to load image:", localPath, err);
    div.innerHTML = `
      <div class="image-placeholder">图片加载失败</div>
      <button type="button" class="remove-btn">×</button>
    `;
  }
  
  div.setAttribute("data-path", localPath);
  container.appendChild(div);
  formDirty = true; // 图片变更也计入脏表单

  div.querySelector(".remove-btn")?.addEventListener("click", () => {
    div.remove();
    formDirty = true;
  });
}

// ============================================================================
// 初始化
// ============================================================================

async function init() {
  // 主题初始化：localStorage 优先，其次系统偏好
  const savedTheme = localStorage.getItem("prefdb-theme");
  if (savedTheme === "dark") {
    document.documentElement.setAttribute("data-theme", "dark");
  } else if (savedTheme === "light") {
    document.documentElement.setAttribute("data-theme", "light");
  } else if (window.matchMedia("(prefers-color-scheme: dark)").matches) {
    document.documentElement.setAttribute("data-theme", "dark");
  }
  const btnThemeInit = document.getElementById("btn-theme");
  if (btnThemeInit && document.documentElement.getAttribute("data-theme") === "dark") {
    btnThemeInit.textContent = "☀️";
  }

  // 视图偏好恢复
  const savedView = localStorage.getItem("prefdb-view");
  if (savedView === "list") {
    const listEl = document.getElementById("entry-list");
    if (listEl) listEl.className = "entry-list list-view";
    document.getElementById("view-cards")?.classList.remove("active");
    document.getElementById("view-list")?.classList.add("active");
  }
  document.getElementById("view-cards")?.setAttribute("aria-pressed", savedView === "list" ? "false" : "true");
  document.getElementById("view-list")?.setAttribute("aria-pressed", savedView === "list" ? "true" : "false");

  // 全局 Esc 关闭最上层模态框（从后往前找，保证先关最上层；与脏表单确认弹窗叠加时行为正确）
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    const modalIds = [
      "modal-cover-pick",
      "modal-cover-source",
      "modal-entry",
      "modal-detail",
      "modal-export",
      "modal-stats",
      "modal-confirm",
      "modal-genre",
    ];
    for (let i = modalIds.length - 1; i >= 0; i--) {
      const id = modalIds[i];
      if (!$<HTMLDivElement>(id).classList.contains("hidden")) {
        closeModal(id);
        break;
      }
    }
  });

  // 焦点陷阱：Tab 在弹窗内循环（无障碍）
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Tab") return;
    const openModals = Array.from(document.querySelectorAll(".modal")).filter(
      (m) => !m.classList.contains("hidden")
    );
    const top = openModals[openModals.length - 1] as HTMLElement | undefined;
    if (!top) return;
    const focusables = Array.from(
      top.querySelectorAll<HTMLElement>(
        'input:not([type="hidden"]), select, textarea, button, [tabindex]:not([tabindex="-1"])'
      )
    ).filter((el) => !el.hasAttribute("disabled"));
    if (focusables.length === 0) return;
    const first = focusables[0];
    const last = focusables[focusables.length - 1];
    const active = document.activeElement;
    if (!top.contains(active)) {
      e.preventDefault();
      first.focus();
    } else if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  });

  // 快捷键（无模态框打开时生效）：Ctrl+N 新增、Ctrl+F 聚焦搜索
  document.addEventListener("keydown", (e) => {
    if (!(e.ctrlKey || e.metaKey)) return;
    const anyOpen = Array.from(document.querySelectorAll(".modal")).some(
      (m) => !m.classList.contains("hidden")
    );
    if (anyOpen) return;
    if (e.key.toLowerCase() === "n") {
      e.preventDefault();
      $("btn-new").dispatchEvent(new MouseEvent("click"));
    } else if (e.key.toLowerCase() === "f") {
      e.preventDefault();
      $("search-input").focus();
    }
  });

  bindEvents();
  await loadGenres();
  await loadTagFilter();
  await loadYearFilter();
  await loadEntries();
  updateBatchButton();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
