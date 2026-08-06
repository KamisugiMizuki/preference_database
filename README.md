# Preference Database · 文艺作品品鉴管理

一个本地优先的**个人文艺作品品鉴管理程序**：记录、整理、回顾你欣赏过的游戏、音乐、动漫、小说、影视剧等作品。所有数据离线存储在本地 SQLite，仅封面爬取需要网络。

基于 **Tauri 2 + Vanilla TypeScript + Vite** 构建，Rust 后端，无前端框架依赖。

## 功能特性

### 条目管理
- **新增/编辑/删除**：作品名称、类型、创作者、评价等级（S/A/B/C）、评价文段（10~20000 字）、品鉴日期、外部链接、多标签、多图片
- **卡片/列表双视图**：卡片显示主图缩略图、彩色等级标签、类型、评价前 50 字
- **详情弹窗**：大图浏览 + 缩略图切换、标签、外链跳转

### 搜索与筛选
- 全字段模糊搜索，或限定于名称 / 标签 / 评价文段
- 按类型、等级组合筛选（支持自定义类型）
- 按名称 / 等级 / 品鉴日期 / 修改时间升降序排序

### 封面爬取
- 多数据源：Bing 图片搜索（通用）、豆瓣（影视/图书）、IMDB（影视）、Bangumi、AniList（动漫）、iTunes（音乐）、Steam（游戏）
- 搜索 → 缩略图网格 → 点击下载到本地 `resource/cover_image/`
- 后备方案：拖放本地图片导入、手动输入路径

### 数据管理
- 导出 JSON / CSV（全库、当前筛选结果或当前列表）
- 手动数据库备份（界面按钮一键完成）

### 界面
- 暗色模式（跟随系统，可手动切换）
- 全程中文界面，数据库支持任意语言字符

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面壳 | Tauri 2（WebView2） |
| 前端 | Vanilla TypeScript + Vite（无框架） |
| 后端 | Rust（rusqlite，参数化查询） |
| 存储 | SQLite（本地离线） |
| 爬取 | reqwest + scraper（同步阻塞实现） |

## 快速开始

### 环境要求
- Node.js ≥ 18
- Rust（stable）+ Cargo
- Windows / macOS / Linux（WebView2 / WKWebView / WebKitGTK）

### 运行

```bash
npm install
npm run tauri dev
```

Windows 下也可以直接双击 `start.bat`（会自动清理残留进程后启动开发模式）。

### 构建发布版

```bash
npm run build && npm run tauri build
```

## 数据与隐私

- **所有数据完全离线存储**：SQLite 数据库位于 `database/database.db`
- 图片保存在 `resource/cover_image/`，数据库仅记录路径
- 以下目录/文件包含个人数据，**不纳入版本控制**（已在 `.gitignore` 中排除）：
  - `database/` — 品鉴记录全文（含个人评价）
  - `backups/` — 数据库备份
  - `exports/` — 导出文件
  - `resource/cover_image/` — 本地封面图片
  - `待玩列表.md` — 个人待玩/待读清单
- 建议定期点击界面右上角 💾 备份数据库，并复制到外部存储

## 项目结构

```
preference_database/
├── src/                  # 前端（TS + CSS）
│   ├── main.ts           # 界面逻辑与状态管理
│   ├── api.ts            # Tauri invoke 封装
│   ├── types.ts          # 数据类型定义
│   └── styles.css        # 样式（含暗色模式）
├── src-tauri/            # Rust 后端
│   └── src/lib.rs        # 数据库、命令、封面爬取实现（单文件）
├── database/             # SQLite 数据库（本地，不入库）
├── backups/              # 数据库备份（本地，不入库）
├── exports/              # 导出文件（本地，不入库）
└── resource/cover_image/ # 封面图片（本地，不入库）
```

## 规划中

- 批量删除、批量爬图（多选 → 逐条选择封面，含"使用第一张 / 全部跳过"）
- JSON / CSV 批量导入
- Markdown / HTML 导出、分享卡片
- 程序关闭时自动备份
- 统计面板（等级分布、时间线）、随机推荐
