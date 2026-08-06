import "./styles.css";
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
let currentEntry: Entry | null = null;
let originalImageIds: string[] = [];
let selectedGenreIds: string[] = [];
let selectedRatings: string[] = ["S", "A", "B", "C"];
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
  setTimeout(() => {
    toastEl.classList.add("hidden");
  }, 3000);
}

function openModal(id: string) {
  $(id).classList.remove("hidden");
}

function closeModal(id: string) {
  $(id).classList.add("hidden");
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

async function renderEntries() {
  const entryListEl = $<HTMLDivElement>("entry-list");
  const entryCountEl = $<HTMLSpanElement>("entry-count");
  const emptyStateEl = $<HTMLDivElement>("empty-state");

  if (entries.length === 0) {
    entryListEl.innerHTML = "";
    entryListEl.classList.add("hidden");
    emptyStateEl.classList.remove("hidden");
    entryCountEl.textContent = "共 0 条作品";
    return;
  }

  entryListEl.classList.remove("hidden");
  emptyStateEl.classList.add("hidden");
  entryCountEl.textContent = `共 ${entries.length} 条作品`;

  const imagePromises = entries.map(async (e) => {
    if (e.primary_image) {
      try {
        console.log("[DEBUG] Loading image for entry:", e.name);
        console.log("[DEBUG] Image path:", e.primary_image);
        console.log("[DEBUG] Path length:", e.primary_image.length);
        const base64 = await api.getImageBase64(e.primary_image);
        const ext = e.primary_image.split(".").pop()?.toLowerCase() || "jpg";
        const dataUrl = `data:image/${ext};base64,${base64}`;
        console.log("[DEBUG] Image loaded, base64 length:", base64.length, "dataUrl length:", dataUrl.length);
        console.log("[DEBUG] First 100 chars of dataUrl:", dataUrl.substring(0, 100));
        return dataUrl;
      } catch (err) {
        console.error("[DEBUG] Failed to load image:", e.primary_image, err);
        return null;
      }
    }
    return null;
  });

  const images = await Promise.all(imagePromises);

  entryListEl.innerHTML = entries
    .map(
      (e, idx) => `
    <div class="entry-card" data-id="${e.id}">
      ${
        images[idx]
          ? `<img class="entry-card-image" src="${images[idx]}" alt="${e.name}" />`
          : `<div class="entry-card-placeholder">${getGenreIcon(e.genre_name)}</div>`
      }
      <div class="entry-card-content">
        <div class="entry-card-header">
          <span class="entry-card-title">${e.name}</span>
          <span class="rating-badge ${e.rating}">${e.rating}</span>
          <span class="entry-card-genre">${e.genre_name}</span>
        </div>
        <p class="entry-card-preview">${e.review_preview}</p>
        ${
          e.tags.length > 0
            ? `<div class="entry-card-tags">
          ${e.tags
            .slice(0, 3)
            .map((t) => `<span class="entry-tag">${t}</span>`)
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

  entryListEl.querySelectorAll(".entry-card").forEach((card) => {
    card.addEventListener("click", () => {
      const id = card.getAttribute("data-id")!;
      showEntryDetail(id);
    });
  });
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
      ? `<img src="${images[idx]}" alt="${entry.name}" />`
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
         data-path="${img.path}" />
  `
    )
    .join("");

  const tagsEl = $("detail-tags");
  tagsEl.innerHTML = entry.tags
    .map((t) => `<span class="detail-tag">${t}</span>`)
    .join("");

  const linksEl = $("detail-links");
  linksEl.innerHTML = entry.links
    .map(
      (l) => `
    <a class="detail-link" href="${l.url}" target="_blank">${l.label || l.url}</a>
  `
    )
    .join("");

  thumbnails.querySelectorAll(".thumbnail").forEach((thumb, idx) => {
    thumb.addEventListener("click", () => {
      mainImage.innerHTML = images[idx] 
        ? `<img src="${images[idx]}" alt="${entry.name}" />`
        : `<div class="placeholder">${getGenreIcon($("detail-genre").textContent || "")}</div>`;
      thumbnails.querySelectorAll(".thumbnail").forEach((t) => t.classList.remove("active"));
      thumb.classList.add("active");
    });
  });
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

async function loadEntries() {
  const loadingEl = $<HTMLDivElement>("loading");
  const entryListEl = $<HTMLDivElement>("entry-list");
  const emptyStateEl = $<HTMLDivElement>("empty-state");

  loadingEl.classList.remove("hidden");
  entryListEl.classList.add("hidden");
  emptyStateEl.classList.add("hidden");

  try {
    currentSearchQuery.genre_ids = selectedGenreIds;
    currentSearchQuery.ratings = selectedRatings;

    entries = await api.getEntries(currentSearchQuery);
    await renderEntries();
  } catch (err) {
    console.error("Failed to load entries:", err);
    showToast("加载作品失败", "error");
  } finally {
    loadingEl.classList.add("hidden");
  }
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
      <input type="text" class="link-label" value="${l.label}" placeholder="标签" />
      <input type="url" class="link-url" value="${l.url}" placeholder="URL" />
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
    <div class="image-item" data-id="${img.id}" data-path="${img.path}">
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
        console.log("[DEBUG] Remove existing image from DOM:", item.getAttribute("data-id"));
        item.remove();
      }
    });
  });
}

async function handleEntrySubmit(e: Event) {
  e.preventDefault();

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
    document.documentElement.setAttribute(
      "data-theme",
      isDark ? "" : "dark"
    );
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

  // 删除按钮
  $("btn-delete-entry").addEventListener("click", () => {
    if (!currentEntry) return;
    $("confirm-message").textContent = `确定要删除《${currentEntry.name}》吗？此操作不可撤销。`;
    openModal("modal-confirm");
  });

  $("btn-confirm-delete").addEventListener("click", async () => {
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

  // 点击模态框背景关闭
  document.querySelectorAll(".modal").forEach((modal) => {
    modal.addEventListener("click", (e) => {
      if (e.target === modal) {
        modal.classList.add("hidden");
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

  // 添加图片（手动输入路径）
  $("btn-add-image").addEventListener("click", async () => {
    const path = prompt("请输入图片路径：");
    if (path) {
      const container = $<HTMLDivElement>("images-container");
      const div = document.createElement("div");
      div.className = "image-item";
      div.setAttribute("data-path", path);
      
      try {
        const base64 = await api.getImageBase64(path);
        const ext = path.split(".").pop()?.toLowerCase() || "jpg";
        const imgSrc = `data:image/${ext};base64,${base64}`;
        div.innerHTML = `
          <img src="${imgSrc}" />
          <button type="button" class="remove-btn">×</button>
        `;
      } catch (err) {
        console.error("[DEBUG] Failed to load local image:", path, err);
        div.innerHTML = `
          <div class="image-placeholder">无法加载图片</div>
          <button type="button" class="remove-btn">×</button>
        `;
      }
      
      container.appendChild(div);

      div.querySelector(".remove-btn")?.addEventListener("click", () => {
        div.remove();
      });
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

  // 联网搜索封面
  $("btn-fetch-cover").addEventListener("click", () => {
    showCoverSourceModal();
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
    openModal("modal-export");
  });

  $("export-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const scope = (document.querySelector('input[name="export-scope"]:checked') as HTMLInputElement).value;
    const format = $<HTMLSelectElement>("export-format").value;
    const includeImages = $<HTMLInputElement>("export-images").checked;

    let ids: string[] | null = null;
    if (scope === "selected") {
      ids = entries.map((e) => e.id);
    }

    try {
      const filePath = await api.exportEntries(ids, format, includeImages);
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

  // 导入（简化实现：触发文件选择）
  $("btn-import").addEventListener("click", () => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,.csv";
    input.onchange = async (e) => {
      const file = (e.target as HTMLInputElement).files?.[0];
      if (!file) return;
      showToast("导入功能开发中...", "error");
    };
    input.click();
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
  tmdb_movie: "IMDB 图片搜索（TMDB 无 key 时替代）",
  bangumi_anime: "Bangumi 番组计划，动画番剧封面",
  anilist_anime: "AniList，动画番剧封面",
  douban_book: "豆瓣读书，图书封面",
  itunes_music: "iTunes 商店，专辑封面",
  igdb_game: "Steam 商店（IGDB 无 key 时替代）",
  steam_game: "Steam 商店，游戏封面",
};

let coverSources: api.CoverSource[] = [];

async function showCoverSourceModal() {
  const name = $<HTMLInputElement>("entry-name").value.trim();
  if (!name) {
    showToast("请先填写作品名称", "error");
    return;
  }

  try {
    if (coverSources.length === 0) {
      coverSources = await api.getCoverSources();
    }
    renderCoverSourceList();
    openModal("modal-cover-source");
  } catch (err) {
    console.error("Failed to load cover sources:", err);
    showToast("加载数据源失败: " + (err as Error).message, "error");
  }
}

function renderCoverSourceList() {
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
      fetchAndShowCandidates(id, source.name);
    });
  });
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
      if (candidate) downloadAndAddCover(candidate);
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
  console.log("[DEBUG] addImageToContainer called with:", localPath);
  const container = $<HTMLDivElement>("images-container");
  const div = document.createElement("div");
  div.className = "image-item";
  
  try {
    const base64 = await api.getImageBase64(localPath);
    const ext = localPath.split(".").pop()?.toLowerCase() || "jpg";
    const imgSrc = `data:image/${ext};base64,${base64}`;
    console.log("[DEBUG] Image loaded, base64 length:", base64.length);
    div.innerHTML = `
      <img src="${imgSrc}" />
      <button type="button" class="remove-btn">×</button>
    `;
  } catch (err) {
    console.error("[DEBUG] Failed to load image:", localPath, err);
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
  const isDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
  if (isDark) {
    document.documentElement.setAttribute("data-theme", "dark");
    const btnTheme = document.getElementById("btn-theme");
    if (btnTheme) btnTheme.textContent = "☀️";
  }

  bindEvents();
  await loadGenres();
  await loadEntries();
}

if (document.readyState === "loading") {
  document.addEventListener("DOMContentLoaded", init);
} else {
  init();
}
