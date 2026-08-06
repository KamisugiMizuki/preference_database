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

function openModal(id: string) {
  $(id).classList.remove("hidden");
}

function closeModal(id: string) {
  $(id).classList.add("hidden");
  // 批量爬图中关闭选择弹窗 → 终止批量
  if (id === "modal-cover-pick" && batchResolve) {
    batchActive = false;
    const r = batchResolve;
    batchResolve = null;
    r();
  }
  // 批量模式下关闭来源选择弹窗 → 取消批量
  if (id === "modal-cover-source" && batchActive) {
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
      <label for="genre-${g.id}">${getGenreIcon(g.name)} ${g.name}</label>
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
  音乐: "#ff6b6b",
  动漫: "#748ffc",
  小说: "#69db7c",
  影视剧: "#ffa94d",
};

function getGenreColor(name: string): string {
  return GENRE_COLORS[name] || "#868e96";
}

/// 加载一批条目的主图 base64
async function loadEntryImages(list: api.EntrySummary[]): Promise<(string | null)[]> {
  return Promise.all(
    list.map(async (e) => {
      if (!e.primary_image) return null;
      try {
        const base64 = await api.getImageBase64(e.primary_image);
        const ext = e.primary_image.split(".").pop()?.toLowerCase() || "jpg";
        return `data:image/${ext};base64,${base64}`;
      } catch (err) {
        console.error("Failed to load image:", e.primary_image, err);
        return null;
      }
    })
  );
}

/// 渲染一批条目卡片（append 时只追加不重绘）
function renderEntryCards(list: api.EntrySummary[], images: (string | null)[], append = false) {
  const entryListEl = $<HTMLDivElement>("entry-list");
  const html = list
    .map(
      (e, idx) => `
    <div class="entry-card" data-id="${e.id}" tabindex="0" role="button" aria-label="查看《${escapeHtml(e.name)}》详情">
      <input type="checkbox" class="entry-select" data-id="${e.id}" ${
        selectedEntryIds.has(e.id) ? "checked" : ""
      } />
      ${
        images[idx]
          ? `<img class="entry-card-image" src="${images[idx]}" alt="${escapeHtml(e.name)}" />`
          : `<div class="entry-card-placeholder" style="background:${getGenreColor(
              e.genre_name
            )}22;color:${getGenreColor(e.genre_name)}">${getGenreIcon(e.genre_name)}</div>`
      }
      <div class="entry-card-content">
        <div class="entry-card-header">
          <span class="entry-card-title">${escapeHtml(e.name)}</span>
          <span class="rating-badge ${e.rating}">${e.rating}</span>
          <span class="entry-card-genre">${escapeHtml(e.genre_name)}</span>
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
  // 诚实计数：显示 X / 共 N 条
  entryCountEl.textContent = `显示 ${entries.length} / 共 ${totalCount} 条作品`;
  // 加载更多按钮
  loadMoreWrapEl.classList.toggle("hidden", entries.length >= totalCount);

  const images = await loadEntryImages(entries);
  renderEntryCards(entries, images, false);
}

async function renderDetailModal(entry: Entry) {
  currentEntry = entry;

  $("detail-title").textContent = entry.name;
  $("detail-genre").textContent = entry.genre_id
    ? genres.find((g) => g.id === entry.genre_id)?.name || ""
    : "";

  const ratingEl = $("detail-rating");
  ratingEl.className = `rating-badge ${entry.rating}`;
  ratingEl.textContent = entry.rating;

  $("detail-date").textContent = formatDate(entry.tasting_date);
  $("detail-creator").textContent = entry.creator || "";
  $("detail-review").textContent = entry.review;

  const imagePromises = entry.images.map(async (img) => {
    try {
      const base64 = await api.getImageBase64(img.path);
      const ext = img.path.split(".").pop()?.toLowerCase() || "jpg";
      return `data:image/${ext};base64,${base64}`;
    } catch {
      return null;
    }
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
         data-path="${escapeHtml(img.path)}" />
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

  // 等级 + 类型/创作者/日期（与主 UI rating 色保持一致：styles.css --rating-*）
  const ratingColors: Record<string, string> = {
    S: "#ff6b6b",
    A: "#ffa94d",
    B: "#69db7c",
    C: "#74c0fc",
  };
  ctx.fillStyle = ratingColors[entry.rating] || "#a6adc8";
  ctx.font = "bold 24px 'Microsoft YaHei', sans-serif";
  ctx.fillText(entry.rating, 32, titleY + 54);
  const genreName = genres.find((g) => g.id === entry.genre_id)?.name || "";
  const metaText = `${genreName}${entry.creator ? " · " + entry.creator : ""}${
    entry.tasting_date ? " · " + formatDate(entry.tasting_date) : ""
  }`;
  ctx.fillStyle = "#a6adc8";
  ctx.font = "16px 'Microsoft YaHei', sans-serif";
  ctx.fillText(metaText, 92, titleY + 54);

  // 评价
  ctx.fillStyle = "#cdd6f4";
  ctx.font = "16px 'Microsoft YaHei', sans-serif";
  drawWrappedText(ctx, entry.review, 32, titleY + 104, W - 64, 28, 12);

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
      const images = await loadEntryImages(newEntries);
      renderEntryCards(newEntries, images, true);
      renderEntries();
    } else {
      totalCount = count;
      entries = newEntries;
      await renderEntries();
    }
  } catch (err) {
    console.error("Failed to load entries:", err);
    showToast("加载作品失败", "error");
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

async function showEntryDetail(id: string) {
  try {
    const entry = await api.getEntry(id);
    await renderDetailModal(entry);
    openModal("modal-detail");
  } catch (err) {
    console.error("Failed to load entry:", err);
    showToast("加载详情失败", "error");
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
}

async function populateEntryForm(entry: Entry) {
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
    try {
      const base64 = await api.getImageBase64(img.path);
      const ext = img.path.split(".").pop()?.toLowerCase() || "jpg";
      return `data:image/${ext};base64,${base64}`;
    } catch {
      return null;
    }
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
      ? tagsStr.split(",").map((t) => t.trim()).filter(Boolean)
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

    closeModal("modal-entry");
    resetEntryForm();
    loadEntries();
  } catch (err) {
    console.error("Failed to save entry:", err);
    showToast("保存失败: " + (err as Error).message, "error");
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
      showToast("生成失败: " + (err as Error).message, "error");
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
      showToast("删除失败: " + (err as Error).message, "error");
    }
  });

  // 表单提交
  $("entry-form").addEventListener("submit", handleEntrySubmit);

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
  $("view-cards").addEventListener("click", () => {
    const entryListEl = $<HTMLDivElement>("entry-list");
    entryListEl.className = "entry-list card-view";
    $("view-cards").classList.add("active");
    $("view-list").classList.remove("active");
  });

  $("view-list").addEventListener("click", () => {
    const entryListEl = $<HTMLDivElement>("entry-list");
    entryListEl.className = "entry-list list-view";
    $("view-list").classList.add("active");
    $("view-cards").classList.remove("active");
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

    div.querySelector(".remove-link")?.addEventListener("click", () => {
      div.remove();
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
      showToast("导入失败: " + (err as Error).message, "error");
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
        showToast("导入失败: " + (err as Error).message, "error");
      }
    }
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
      showToast("添加失败: " + (err as Error).message, "error");
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
      showToast("导出失败: " + (err as Error).message, "error");
    }
  });

  // 备份
  $("btn-backup").addEventListener("click", async () => {
    try {
      const path = await api.backupDatabase();
      showToast(`备份成功: ${path}`);
    } catch (err) {
      showToast("备份失败: " + (err as Error).message, "error");
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
        showToast("导入失败: " + (err as Error).message, "error");
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
      showToast("导入失败: " + (err as Error).message, "error");
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
    showToast("加载数据源失败: " + (err as Error).message, "error");
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
    <div class="source-item" data-id="${s.id}">
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
      closeModal("modal-cover-source");
      if (batch) {
        batchSourceId = id;
        batchSourceName = source.name;
        processBatchQueue();
      } else {
        fetchAndShowCandidates(id, source.name);
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
      grid.innerHTML = `<div class="cover-empty">搜索失败：${(err as Error).message}</div>`;
    }

    // 等待用户操作（点图 / 使用第一张 / 跳过 / 全部跳过 / 关闭）
    await new Promise<void>((resolve) => {
      batchResolve = resolve;
    });
    batchResolve = null;
  }

  // 批量结束
  batchActive = false;
  batchCurrentItem = null;
  batchCurrentCandidates = [];
  $("btn-cover-change-source").classList.remove("hidden");
  $("btn-cover-first").classList.add("hidden");
  $("btn-cover-skip").classList.add("hidden");
  $("btn-cover-skip-all").classList.add("hidden");
  closeModal("modal-cover-pick");
  selectedEntryIds.clear();
  updateBatchButton();
  showToast("批量爬图完成");
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
    showToast(`《${item.name}》封面下载失败: ${(err as Error).message}`, "error");
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
    grid.innerHTML = `<div class="cover-empty">搜索失败：${(err as Error).message}</div>`;
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
    <div class="cover-cell" data-idx="${idx}">
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
    showToast("下载失败: " + (err as Error).message, "error");
  }
}

async function addImageToContainer(localPath: string) {
  const container = $<HTMLDivElement>("images-container");
  const div = document.createElement("div");
  div.className = "image-item";
  
  try {
    const base64 = await api.getImageBase64(localPath);
    const ext = localPath.split(".").pop()?.toLowerCase() || "jpg";
    const imgSrc = `data:image/${ext};base64,${base64}`;
    div.innerHTML = `
      <img src="${imgSrc}" />
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

  div.querySelector(".remove-btn")?.addEventListener("click", () => {
    div.remove();
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

  // 全局 Esc 关闭最上层模态框（无障碍）
  document.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    const modalIds = [
      "modal-cover-pick",
      "modal-cover-source",
      "modal-entry",
      "modal-detail",
      "modal-export",
      "modal-confirm",
      "modal-genre",
    ];
    for (const id of modalIds) {
      if (!$<HTMLDivElement>(id).classList.contains("hidden")) {
        closeModal(id);
        break;
      }
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
