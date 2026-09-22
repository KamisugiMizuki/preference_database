# Preference Database · 文艺作品品鉴管理

一个本地优先的**个人文艺作品品鉴管理程序**：记录、整理、回顾你欣赏过的游戏、音乐、动漫、小说、影视剧等作品。所有数据离线存储在本地 SQLite，仅封面爬取需要网络。

基于 **Tauri 2 + Vanilla TypeScript + Vite** 构建，Rust 后端，无前端框架依赖。

## 功能特性

### 条目管理
- **新增/编辑/删除**：作品名称、类型、创作者、评价等级（S/A/B/C）、评价文段（10~20000 字）、品鉴日期、外部链接、多标签、多图片；删除支持单个与批量（均二次确认，同步清理图片文件），编辑弹窗带未保存修改保护
- **卡片/列表双视图**：卡片显示主图缩略图、彩色等级标签、类型、评价前 50 字；视图偏好持久化
- **详情弹窗**：大图浏览 + 缩略图切换、标签、外链跳转

### 搜索与筛选
- 全字段模糊搜索，或限定于名称 / 标签 / 评价文段
- 按类型、等级、标签、品鉴年份组合筛选（支持自定义类型）
- 按名称 / 等级 / 品鉴日期 / 修改时间升降序排序

### 封面爬取
- 多数据源：Bing 图片搜索（通用）、豆瓣（影视/图书）、Bangumi、AniList（动漫）、iTunes（音乐）、Steam（游戏）
- 搜索 → 缩略图网格 → 点击下载到本地 `resource/cover_image/`
- **批量爬图**：列表多选条目后逐条选择封面，支持「使用第一张 / 跳过本条目 / 全部跳过」
- 搜索、候选预览、下载及重定向共用按主机至少 1 秒的请求间隔；图片校验 HTTP 状态、MIME 和文件头，单图上限 10 MiB
- 后备方案：原生文件对话框选择本地图片导入
- Bangumi 源支持本地 Cookie 配置（R18 内容随账号权限，见下文）

### 数据管理
- 导出 JSON / CSV / Markdown / HTML（HTML 固定嵌入图片；其他格式勾选包含图片时，副本保存在本次导出目录，使用相对路径）
- 导入 JSON / CSV（类型缺失自动创建；每条记录独立事务，失败不留半条数据；图片复制成独立文件，支持重复导入）
- CSV 保持原八列兼容，新增可选“图片”列，内容为路径的 JSON 数组；跨设备转移时请保留整个导出目录
- 手动备份 / 恢复数据库（使用 SQLite 在线备份，包含已提交的 WAL 数据；恢复前自动备份当前库）
- 关闭程序时自动备份
- 单条目分享卡片（生成 PNG 图片）
- 统计面板（等级 / 类型 / 品鉴年份分布）

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

- **所有数据完全离线存储**：开发树内运行沿用项目根目录；Windows 安装版使用 `%LOCALAPPDATA%/Preference Database/`（macOS/Linux 使用相应用户数据目录）
- 数据目录中数据库为 `database/database.db`，图片为 `resource/cover_image/`，备份为 `backups/`，导出为 `exports/`
- 首次安装版启动会检查旧版位置并迁移数据，原文件保留；已有用户数据库优先使用
- 图片仅在数据库提交删除且没有其他引用时清理；占用失败会登记并在下次启动、备份或删除操作时重试；旧外部图片原件保留
- 以下目录/文件包含个人数据，**不纳入版本控制**（已在 `.gitignore` 中排除）：
  - `database/` — 品鉴记录全文（含个人评价）
  - `backups/` — 数据库备份
  - `exports/` — 导出文件
  - `resource/cover_image/` — 本地封面图片
- 建议定期点击界面右上角 💾 备份数据库，并复制到外部存储

## Bangumi Cookie 配置（可选）

Bangumi 搜索对**未登录或无权限**账号隐藏全部 R18 内容。默认匿名搜索即可使用（不显示 R18）；如需在 Bangumi 源中搜索 R18 作品封面，可配置你的登录 Cookie：

1. 浏览器登录 [bgm.tv](https://bgm.tv)（账号需具备 R18 内容访问权限，否则服务端仍会隐藏 R18，此为网站策略，无法绕过）
2. 按 F12 打开开发者工具 → 切到 **Network（网络）** 面板 → 刷新页面 → 点击任意一个请求（如首页文档请求）
3. 在右侧 **Headers（标头）→ Request Headers（请求标头）** 中找到 `Cookie` 行 → 右键 → **Copy value**（复制值）
4. 将复制的内容粘贴到本地文件 `config/bangumi_cookie.txt`（单行即可，带不带 `Cookie:` 前缀都能识别）
5. 该文件包含你的登录凭证，**不会提交到 Git**

> **为什么不用控制台一行代码？** bgm.tv 的登录凭证（`chii_auth` 等）标记为 **HttpOnly**，JavaScript 无法读取，控制台运行 `document.cookie` 只能拿到非登录 cookie，对 R18 无效。必须通过 Network 面板复制完整请求头 Cookie（此方法能看到 HttpOnly cookie）。

行为说明：

- 配置了 Cookie：Bangumi 搜索请求携带 Cookie，R18 内容可见（账号满足条件时）
- 未配置 Cookie，或带 Cookie 的请求失败：**自动回退为匿名 Bangumi 搜索**（不显示 R18），不影响正常使用

## 项目结构

```
preference_database/
├── index.html              # 页面骨架（工具栏、列表区、模态框）
├── vite.config.ts          # Vite 配置（dev 端口 5173，绑定 127.0.0.1）
├── tsconfig.json           # TypeScript 配置
├── package.json            # 前端依赖与脚本
├── start.bat               # Windows 一键启动开发模式
├── claude.md               # 项目需求规格说明
├── src/                    # 前端（TS + CSS）
│   ├── main.ts             # 界面逻辑与状态管理
│   ├── api.ts              # Tauri invoke 封装
│   ├── types.ts            # 数据类型定义
│   ├── styles.css          # 样式（含暗色模式）
│   └── assets/             # 静态资源（图标）
├── src-tauri/              # Rust 后端
│   ├── Cargo.toml          # 依赖（tauri/rusqlite/reqwest/scraper 等）
│   ├── tauri.conf.json     # Tauri 配置（窗口、devUrl、打包）
│   ├── build.rs
│   ├── capabilities/
│   │   └── default.json    # 权限声明（dialog/opener）
│   ├── icons/              # 应用图标（各平台尺寸）
│   └── src/
│       ├── main.rs         # 程序入口
│       ├── lib.rs          # 数据库、命令、封面爬取
│       ├── storage.rs      # 数据目录定位与旧数据迁移
│       └── reliability_tests.rs # 事务、图片、网络与备份回归测试
├── config/                 # 本地配置（含 README 说明；Cookie 等凭证不入库）
├── database/               # SQLite 数据库（本地，不入库）
├── backups/                # 数据库备份（本地，不入库）
├── exports/                # 导出文件（本地，不入库）
└── resource/cover_image/   # 封面图片（本地，不入库）
```

## 规划中

- 随机推荐
- 社交分享（生成匿名展示页）、AI 辅助生成评价
