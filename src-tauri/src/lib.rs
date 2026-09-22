use base64::Engine;
use chrono::Utc;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ============================================================================
// 数据库模型
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Genre {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalLink {
    pub id: String,
    pub entry_id: String,
    pub url: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: String,
    pub entry_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryImage {
    pub id: String,
    pub entry_id: String,
    pub path: String,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub genre_id: String,
    pub creator: Option<String>,
    pub rating: String, // S, A, B, C
    pub review: String,
    pub tasting_date: Option<String>,
    pub links: Vec<ExternalLink>,
    pub tags: Vec<String>,
    pub images: Vec<EntryImage>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntrySummary {
    pub id: String,
    pub name: String,
    pub genre_name: String,
    pub rating: String,
    pub review_preview: String,
    pub primary_image: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateEntryRequest {
    pub name: String,
    pub genre_id: String,
    pub creator: Option<String>,
    pub rating: String,
    pub review: String,
    pub tasting_date: Option<String>,
    pub links: Vec<ExternalLink>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub image_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateEntryRequest {
    pub id: String,
    pub name: String,
    pub genre_id: String,
    pub creator: Option<String>,
    pub rating: String,
    pub review: String,
    pub tasting_date: Option<String>,
    pub links: Vec<ExternalLink>,
    pub tags: Vec<String>,
    #[serde(default)]
    pub new_image_paths: Vec<String>,
    #[serde(default)]
    pub removed_image_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub keyword: Option<String>,
    pub search_field: Option<String>, // "all", "name", "tags", "review"
    pub genre_ids: Vec<String>,
    pub ratings: Vec<String>,
    pub tag_filter: Vec<String>,
    pub year: Option<i32>,
    pub sort_by: String,    // "name", "rating", "tasting_date", "updated_at"
    pub sort_order: String, // "asc", "desc"
    pub offset: i64,
    pub limit: i64,
}

// ============================================================================
// 封面爬取相关结构
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverSource {
    pub id: String,
    pub name: String,
    pub source_type: String, // "douban", "bing", "google", "bangumi" 等
    pub usage: String,       // "general", "movie", "book", "music", "anime", "game"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverCandidate {
    pub url: String,
    pub thumbnail_url: Option<String>,
    pub title: Option<String>,
    pub source: String, // 来源 ID
    pub width: Option<u32>,
    pub height: Option<u32>,
}

// ============================================================================
// 数据库初始化
// ============================================================================

#[cfg(test)]
fn get_project_root() -> std::path::PathBuf {
    static ROOT: Lazy<std::path::PathBuf> = Lazy::new(|| {
        let root = std::env::temp_dir().join(format!("prefdb_tests_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        root
    });
    ROOT.clone()
}

mod storage;

#[cfg(not(test))]
fn get_project_root() -> std::path::PathBuf {
    static ROOT: Lazy<std::path::PathBuf> = Lazy::new(|| {
        let exe = std::env::current_exe().expect("无法定位程序路径");
        let local = dirs::data_local_dir().expect("无法定位用户数据目录");
        storage::prepare_data_root(&exe, &local.join("Preference Database"))
            .unwrap_or_else(|error| panic!("数据目录初始化失败，原数据未删除: {}", error))
    });
    ROOT.clone()
}

/// 图片路径解析：相对路径拼接项目根，绝对路径（旧数据）原样返回
fn resolve_image_path(path: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        get_project_root().join(p)
    }
}

fn validate_http_url(raw: &str) -> Result<(), String> {
    if raw.chars().any(char::is_control) {
        return Err("URL 不能含控制字符".to_string());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| "只允许有效的 HTTP(S) URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("只允许 HTTP(S) URL".to_string());
    }
    Ok(())
}

fn request_host(raw: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(raw).map_err(|_| "无效的请求 URL".to_string())?;
    Ok(url
        .host_str()
        .ok_or_else(|| "请求 URL 缺少主机名".to_string())?
        .to_ascii_lowercase())
}

// ponytail: 桌面端串行发送；需要并行抓取时再拆成每主机调度。
static HTTP_DISPATCH: Mutex<()> = Mutex::new(());
static LAST_REQUEST_BY_HOST: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn send_rate_limited(
    request: reqwest::blocking::RequestBuilder,
    url: &str,
) -> Result<reqwest::blocking::Response, String> {
    let _dispatch = HTTP_DISPATCH.lock().map_err(|e| e.to_string())?;
    wait_for_request_slot(url)?;
    let response = request.send().map_err(|e| format!("请求失败: {}", e))?;
    LAST_REQUEST_BY_HOST
        .lock()
        .map_err(|e| e.to_string())?
        .insert(request_host(response.url().as_str())?, Instant::now());
    response
        .error_for_status()
        .map_err(|e| format!("服务端返回错误: {}", e))
}

fn wait_for_request_slot(url: &str) -> Result<(), String> {
    validate_http_url(url)?;
    let host = request_host(url)?;
    let mut last_requests = LAST_REQUEST_BY_HOST
        .lock()
        .map_err(|e| format!("请求限速器不可用: {}", e))?;
    if let Some(last) = last_requests.get(&host) {
        let elapsed = last.elapsed();
        if elapsed < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_secs(1) - elapsed);
        }
    }
    last_requests.insert(host, Instant::now());
    Ok(())
}

fn image_extension_from_bytes(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else if bytes.starts_with(b"BM") {
        Some("bmp")
    } else {
        None
    }
}

fn validate_image_bytes(bytes: &[u8]) -> Result<&'static str, String> {
    const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
    if bytes.is_empty() {
        return Err("图片内容为空".to_string());
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("图片超过 10 MiB 大小限制".to_string());
    }
    image_extension_from_bytes(bytes).ok_or_else(|| "下载内容不是受支持的图片格式".to_string())
}

fn validate_image_response(
    response: reqwest::blocking::Response,
) -> Result<(Vec<u8>, &'static str), String> {
    const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        content_type.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp" | "image/bmp" | "image/x-ms-bmp"
    ) {
        return Err("响应不是受支持的图片类型".to_string());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES)
    {
        return Err("图片超过 10 MiB 大小限制".to_string());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取图片失败: {}", e))?;
    let extension = validate_image_bytes(&bytes)?;
    let declared_extension = match content_type.as_str() {
        "image/jpeg" => "jpg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" | "image/x-ms-bmp" => "bmp",
        _ => unreachable!(),
    };
    if extension != declared_extension {
        return Err("图片内容与响应类型不一致".to_string());
    }
    Ok((bytes, extension))
}

fn validate_external_links(links: &[ExternalLink]) -> Result<(), String> {
    for link in links {
        if !link.url.trim().is_empty() {
            validate_http_url(link.url.trim())?;
        }
    }
    Ok(())
}

/// 新写入的图片只能位于应用自己的封面目录，旧库的绝对路径仍可读取。
fn validate_project_image_path(path: &str) -> Result<std::path::PathBuf, String> {
    use std::path::Component;

    let p = std::path::Path::new(path);
    let cover_dir = std::path::Path::new("resource").join("cover_image");
    if p.is_absolute()
        || !p.starts_with(&cover_dir)
        || p.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("图片路径必须位于 resource/cover_image 目录内".to_string());
    }
    if p.components()
        .any(|part| part.as_os_str().to_string_lossy().contains(':'))
    {
        return Err("图片路径不能包含设备名或数据流".to_string());
    }
    let managed = managed_cover_dir()?;
    let resolved = get_project_root()
        .join(p)
        .canonicalize()
        .map_err(|e| format!("图片不存在: {}", e))?;
    if !resolved.starts_with(&managed) || !resolved.is_file() {
        return Err("图片路径越出封面目录或不是文件".to_string());
    }
    Ok(resolved)
}

fn managed_cover_dir() -> Result<std::path::PathBuf, String> {
    let root = get_project_root()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let dir = root.join("resource/cover_image");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dir = dir.canonicalize().map_err(|e| e.to_string())?;
    if !dir.starts_with(&root) {
        return Err("封面目录不能指向数据目录以外".to_string());
    }
    Ok(dir)
}

fn image_path_key(path: &std::path::Path) -> Option<String> {
    let key = path
        .canonicalize()
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    Some(if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    })
}

fn remove_unreferenced_image_files(paths: &[String]) {
    let Ok(conn) = DB.lock() else {
        return;
    };
    if let Err(error) = retry_image_cleanup(&conn, paths) {
        eprintln!("[WARN] 图片清理将在下次操作重试: {}", error);
    }
}

fn retry_image_cleanup(conn: &Connection, paths: &[String]) -> Result<(), String> {
    for path in paths {
        conn.execute(
            "INSERT OR IGNORE INTO image_cleanup(path) VALUES(?1)",
            params![path],
        )
        .map_err(|e| e.to_string())?;
    }
    let references: Vec<String> = conn
        .prepare("SELECT DISTINCT path FROM entry_images")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<SqliteResult<_>>()
        .map_err(|e| e.to_string())?;
    let referenced: std::collections::HashSet<_> = references
        .iter()
        .filter_map(|path| image_path_key(&resolve_image_path(path)))
        .collect();
    let pending: Vec<String> = conn
        .prepare("SELECT path FROM image_cleanup")
        .map_err(|e| e.to_string())?
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<SqliteResult<_>>()
        .map_err(|e| e.to_string())?;
    let managed = managed_cover_dir()?;
    for path in pending {
        let resolved = resolve_image_path(&path);
        let forget = match resolved.canonicalize() {
            Ok(actual) => {
                if !actual.starts_with(&managed) || !actual.is_file() {
                    // 旧外部文件归用户所有，删除条目不删除外部原件。
                    true
                } else if image_path_key(&actual).is_some_and(|key| referenced.contains(&key)) {
                    true // 新引用重新接管该图片，后续删除仍由触发器登记。
                } else {
                    match std::fs::remove_file(&actual) {
                        Ok(()) => true,
                        Err(error) => {
                            eprintln!(
                                "[WARN] 图片仍被占用，稍后重试 {}: {}",
                                actual.display(),
                                error
                            );
                            false
                        }
                    }
                }
            }
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        if forget {
            conn.execute("DELETE FROM image_cleanup WHERE path=?1", params![path])
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn import_image_source_path(
    raw: &str,
    import_file: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let source = std::path::Path::new(raw);
    if source
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("导入图片路径不能包含 ..".to_string());
    }
    let base = import_file
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let candidate = base.join(source);
    if let Ok(resolved) = candidate.canonicalize() {
        if resolved.starts_with(&base) && resolved.is_file() {
            return Ok(resolved);
        }
    }
    // 兼容旧 JSON 中记录的本应用封面路径；不读取导入文件任意指定的外部文件。
    if source.is_absolute() {
        let resolved = source.canonicalize().map_err(|e| e.to_string())?;
        if resolved.starts_with(managed_cover_dir()?) && resolved.is_file() {
            return Ok(resolved);
        }
    } else if let Ok(resolved) = validate_project_image_path(raw) {
        return Ok(resolved);
    }
    Err(format!("导入图片不存在或越出导入目录: {}", raw))
}

fn read_image_file(path: &std::path::Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| format!("读取图片失败: {}", e))?
        .take(10 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    validate_image_bytes(&bytes)?;
    Ok(bytes)
}

fn store_image_bytes(bytes: &[u8], name: &str) -> Result<String, String> {
    use std::io::Write;
    let ext = validate_image_bytes(bytes)?;
    let stem: String = sanitize_filename(name).chars().take(60).collect();
    let file_name = format!("{}_{}.{}", stem, Uuid::new_v4(), ext);
    let target = managed_cover_dir()?.join(&file_name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&target)
        .map_err(|e| e.to_string())?;
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let _ = std::fs::remove_file(&target);
        return Err(error.to_string());
    }
    Ok(format!("resource/cover_image/{}", file_name))
}

fn copy_imported_images(
    image_paths: &[String],
    import_file: &std::path::Path,
) -> Result<(Vec<String>, Vec<std::path::PathBuf>), String> {
    let mut stored_paths = Vec::new();
    let mut copied_files = Vec::new();
    if image_paths.is_empty() {
        return Ok((stored_paths, copied_files));
    }

    let result = (|| -> Result<(), String> {
        for raw in image_paths.iter().filter(|path| !path.trim().is_empty()) {
            let source = import_image_source_path(raw, import_file)?;
            let bytes = read_image_file(&source)?;
            let stored = store_image_bytes(&bytes, "import")?;
            copied_files.push(resolve_image_path(&stored));
            stored_paths.push(stored);
        }
        Ok(())
    })();
    if let Err(error) = result {
        remove_copied_files(&copied_files);
        return Err(error);
    }
    Ok((stored_paths, copied_files))
}

fn remove_copied_files(paths: &[std::path::PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// 绝对路径转项目相对路径（用于入库）；不在项目内则原样返回
#[cfg(test)]
fn to_project_rel_path(path: &std::path::Path) -> String {
    match path.strip_prefix(get_project_root()) {
        Ok(rel) => rel.to_string_lossy().to_string(),
        Err(_) => path.to_string_lossy().to_string(),
    }
}

fn get_db_path() -> String {
    let project_root = get_project_root();
    let db_dir = project_root.join("database");
    std::fs::create_dir_all(&db_dir).ok();
    db_dir.join("database.db").to_string_lossy().to_string()
}

static DB: Lazy<Mutex<Connection>> = Lazy::new(|| {
    let db_path = get_db_path();
    let conn = Connection::open(&db_path).expect("Failed to open database");
    init_database(&conn).expect("Failed to initialize database");
    if let Err(error) = retry_image_cleanup(&conn, &[]) {
        eprintln!("[WARN] 启动时图片清理失败: {}", error);
    }
    Mutex::new(conn)
});

fn init_database(conn: &Connection) -> SqliteResult<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON")?;

    // 作品类型表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS genres (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            is_default INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        )",
        [],
    )?;

    // 作品条目表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS entries (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            genre_id TEXT NOT NULL,
            creator TEXT,
            rating TEXT NOT NULL,
            review TEXT NOT NULL,
            tasting_date TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (genre_id) REFERENCES genres(id)
        )",
        [],
    )?;

    // 外部链接表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS external_links (
            id TEXT PRIMARY KEY,
            entry_id TEXT NOT NULL,
            url TEXT NOT NULL,
            label TEXT NOT NULL,
            FOREIGN KEY (entry_id) REFERENCES entries(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // 标签表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS tags (
            id TEXT PRIMARY KEY,
            entry_id TEXT NOT NULL,
            name TEXT NOT NULL,
            FOREIGN KEY (entry_id) REFERENCES entries(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // 图片表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS entry_images (
            id TEXT PRIMARY KEY,
            entry_id TEXT NOT NULL,
            path TEXT NOT NULL,
            is_primary INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY (entry_id) REFERENCES entries(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // 与图片引用删除处于同一事务，文件占用或退出后仍可重试。
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS image_cleanup(path TEXT PRIMARY KEY);
        CREATE TRIGGER IF NOT EXISTS queue_image_cleanup AFTER DELETE ON entry_images
        BEGIN INSERT OR IGNORE INTO image_cleanup(path) VALUES(OLD.path); END;",
    )?;

    // 创建索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entries_genre ON entries(genre_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entries_rating ON entries(rating)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_entries_name ON entries(name)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_tags_entry ON tags(entry_id)",
        [],
    )?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tags_name ON tags(name)", [])?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_links_entry ON external_links(entry_id)",
        [],
    )?;

    // 插入默认类型
    let default_genres = ["游戏", "音乐", "动漫", "小说", "影视剧"];
    for genre in default_genres {
        conn.execute(
            "INSERT OR IGNORE INTO genres (id, name, is_default, created_at) VALUES (?1, ?2, 1, ?3)",
            params![Uuid::new_v4().to_string(), genre, Utc::now().to_rfc3339()],
        )?;
    }

    Ok(())
}

// ============================================================================
// 辅助函数
// ============================================================================

/// 校验条目字段（与 CLAUDE.md 需求一致）：名称 ≤200、评价 10~20000、等级枚举、类型存在
fn validate_entry_fields(
    name: &str,
    genre_id: &str,
    rating: &str,
    review: &str,
    conn: &Connection,
) -> Result<(), String> {
    let name_len = name.trim().chars().count();
    if name_len == 0 {
        return Err("作品名称不能为空".to_string());
    }
    if name_len > 200 {
        return Err(format!(
            "作品名称长度不能超过 200 字符（当前 {}）",
            name_len
        ));
    }
    let review_len = review.trim().chars().count();
    if review_len < 10 {
        return Err(format!(
            "个人评价文段至少需要 10 字符（当前 {}）",
            review_len
        ));
    }
    if review_len > 20000 {
        return Err(format!(
            "个人评价文段不能超过 20000 字符（当前 {}）",
            review_len
        ));
    }
    if !["S", "A", "B", "C"].contains(&rating) {
        return Err(format!("无效的评价等级: {}", rating));
    }
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM genres WHERE id = ?1)",
            params![genre_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Err("作品类型不存在".to_string());
    }
    Ok(())
}

// ============================================================================
// 类型管理命令
// ============================================================================

#[tauri::command]
fn get_genres() -> Result<Vec<Genre>, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, name, is_default, created_at FROM genres ORDER BY is_default DESC, name ASC")
        .map_err(|e| e.to_string())?;

    let genres = stmt
        .query_map([], |row| {
            Ok(Genre {
                id: row.get(0)?,
                name: row.get(1)?,
                is_default: row.get::<_, i32>(2)? != 0,
                created_at: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(genres)
}

#[tauri::command]
fn create_genre(name: String) -> Result<Genre, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO genres (id, name, is_default, created_at) VALUES (?1, ?2, 0, ?3)",
        params![id, name, created_at],
    )
    .map_err(|e| e.to_string())?;

    Ok(Genre {
        id,
        name,
        is_default: false,
        created_at,
    })
}

#[tauri::command]
fn delete_genre(id: String) -> Result<(), String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    // 检查是否有条目使用此类型
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE genre_id = ?1",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    if count > 0 {
        return Err("无法删除：此类型下仍有作品条目".to_string());
    }

    conn.execute(
        "DELETE FROM genres WHERE id = ?1 AND is_default = 0",
        params![id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// 条目管理命令
// ============================================================================

#[tauri::command]
/// 构建筛选片段（JOIN + WHERE）与参数，get_entries 与 export_entries 共用
fn build_filter_sql(query: &SearchQuery) -> (String, Vec<Box<dyn rusqlite::ToSql>>) {
    let mut extra = String::new(); // JOIN 片段（追加在 FROM 之后）
    let mut conditions: Vec<String> = vec![];
    let mut params_vec: Vec<Box<dyn rusqlite::ToSql>> = vec![];

    // 关键词搜索
    if let Some(ref keyword) = query.keyword {
        let field = query.search_field.as_deref().unwrap_or("all");
        let pattern = format!("%{}%", keyword);
        match field {
            "name" => {
                conditions.push("e.name LIKE ?".to_string());
                params_vec.push(Box::new(pattern));
            }
            "tags" => {
                extra.push_str(" LEFT JOIN tags t ON e.id = t.entry_id");
                conditions.push("t.name LIKE ?".to_string());
                params_vec.push(Box::new(pattern));
            }
            "review" => {
                conditions.push("e.review LIKE ?".to_string());
                params_vec.push(Box::new(pattern));
            }
            _ => {
                extra.push_str(" LEFT JOIN tags t ON e.id = t.entry_id");
                conditions.push("(e.name LIKE ? OR e.review LIKE ? OR t.name LIKE ?)".to_string());
                params_vec.push(Box::new(pattern.clone()));
                params_vec.push(Box::new(pattern.clone()));
                params_vec.push(Box::new(pattern));
            }
        }
    }

    // 类型筛选
    if !query.genre_ids.is_empty() {
        let placeholders: Vec<String> = query.genre_ids.iter().map(|_| "?".to_string()).collect();
        conditions.push(format!("e.genre_id IN ({})", placeholders.join(",")));
        for gid in &query.genre_ids {
            params_vec.push(Box::new(gid.clone()));
        }
    }

    // 等级筛选
    if !query.ratings.is_empty() {
        let placeholders: Vec<String> = query.ratings.iter().map(|_| "?".to_string()).collect();
        conditions.push(format!("e.rating IN ({})", placeholders.join(",")));
        for r in &query.ratings {
            params_vec.push(Box::new(r.clone()));
        }
    }

    // 标签筛选
    if !query.tag_filter.is_empty() {
        extra.push_str(" INNER JOIN tags t2 ON e.id = t2.entry_id");
        let placeholders: Vec<String> = query.tag_filter.iter().map(|_| "?".to_string()).collect();
        conditions.push(format!("t2.name IN ({})", placeholders.join(",")));
        for tag in &query.tag_filter {
            params_vec.push(Box::new(tag.clone()));
        }
    }

    // 年份筛选
    if let Some(year) = query.year {
        conditions.push("strftime('%Y', e.tasting_date) = ?".to_string());
        params_vec.push(Box::new(year.to_string()));
    }

    let mut sql = extra;
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    (sql, params_vec)
}

#[tauri::command]
fn get_entries(query: SearchQuery) -> Result<Vec<EntrySummary>, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    let mut sql = String::from(
        "SELECT DISTINCT e.id, e.name, g.name as genre_name, e.rating, e.review, e.created_at, e.updated_at
         FROM entries e
         JOIN genres g ON e.genre_id = g.id"
    );

    let (filter_sql, params_vec) = build_filter_sql(&query);
    sql.push_str(&filter_sql);

    // 排序（等级按 S>A>B>C 语义序，非字母序）
    let sort_expr = match query.sort_by.as_str() {
        "name" => "e.name".to_string(),
        "rating" => {
            "CASE e.rating WHEN 'S' THEN 0 WHEN 'A' THEN 1 WHEN 'B' THEN 2 ELSE 3 END".to_string()
        }
        "tasting_date" => "e.tasting_date".to_string(),
        _ => "e.updated_at".to_string(),
    };
    let sort_dir = if query.sort_order == "asc" {
        "ASC"
    } else {
        "DESC"
    };
    // 次级键 e.id 保证同值区间分页稳定（避免 LIMIT/OFFSET 重复或丢条目）
    sql.push_str(&format!(
        " ORDER BY {} {}, e.id {}",
        sort_expr, sort_dir, sort_dir
    ));

    // 分页
    sql.push_str(&format!(" LIMIT {} OFFSET {}", query.limit, query.offset));

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    let entries = stmt
        .query_map(params_refs.as_slice(), |row| {
            let review: String = row.get(4)?;
            Ok(EntrySummary {
                id: row.get(0)?,
                name: row.get(1)?,
                genre_name: row.get(2)?,
                rating: row.get(3)?,
                review_preview: review.chars().take(50).collect(),
                primary_image: None,
                tags: vec![],
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // 补充标签和主图
    drop(stmt);
    let mut result = Vec::new();
    for mut entry in entries {
        let mut stmt = conn
            .prepare("SELECT name FROM tags WHERE entry_id = ?")
            .map_err(|e| e.to_string())?;
        let tags: Vec<String> = stmt
            .query_map(params![entry.id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        let primary_image: Option<String> = conn
            .query_row(
                "SELECT path FROM entry_images WHERE entry_id = ? AND is_primary = 1",
                params![entry.id],
                |row| row.get(0),
            )
            .ok();

        entry.tags = tags;
        entry.primary_image = primary_image;
        result.push(entry);
    }

    Ok(result)
}

#[tauri::command]
fn get_entry(id: String) -> Result<Entry, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    let entry = conn
        .query_row(
            "SELECT id, name, genre_id, creator, rating, review, tasting_date, created_at, updated_at
             FROM entries WHERE id = ?",
            params![id],
            |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    genre_id: row.get(2)?,
                    creator: row.get(3)?,
                    rating: row.get(4)?,
                    review: row.get(5)?,
                    tasting_date: row.get(6)?,
                    links: vec![],
                    tags: vec![],
                    images: vec![],
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    // 获取链接
    let mut stmt = conn
        .prepare("SELECT id, entry_id, url, label FROM external_links WHERE entry_id = ?")
        .map_err(|e| e.to_string())?;
    let links = stmt
        .query_map(params![id], |row| {
            Ok(ExternalLink {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                url: row.get(2)?,
                label: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    // 获取标签
    let mut stmt = conn
        .prepare("SELECT name FROM tags WHERE entry_id = ?")
        .map_err(|e| e.to_string())?;
    let tags = stmt
        .query_map(params![id], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    // 获取图片
    let mut stmt = conn
        .prepare("SELECT id, entry_id, path, is_primary FROM entry_images WHERE entry_id = ?")
        .map_err(|e| e.to_string())?;
    let images = stmt
        .query_map(params![id], |row| {
            Ok(EntryImage {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                path: row.get(2)?,
                is_primary: row.get::<_, i32>(3)? != 0,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(Entry {
        links,
        tags,
        images,
        ..entry
    })
}

#[tauri::command]
fn create_entry(req: CreateEntryRequest) -> Result<Entry, String> {
    let mut conn = DB.lock().map_err(|e| e.to_string())?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let image_paths = req.image_paths.clone();

    let result = (|| -> Result<(), String> {
        validate_entry_fields(&req.name, &req.genre_id, &req.rating, &req.review, &conn)?;
        validate_external_links(&req.links)?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO entries (id, name, genre_id, creator, rating, review, tasting_date, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                req.name,
                req.genre_id,
                req.creator,
                req.rating,
                req.review,
                req.tasting_date,
                now,
                now
            ],
        )
        .map_err(|e| e.to_string())?;

        for link in &req.links {
            let link_id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
                params![link_id, id, link.url, link.label],
            )
            .map_err(|e| e.to_string())?;
        }

        for tag in &req.tags {
            let tag_id = Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
                params![tag_id, id, tag],
            )
            .map_err(|e| e.to_string())?;
        }

        for (index, path) in image_paths.iter().enumerate() {
            validate_project_image_path(path)?;
            tx.execute(
                "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
                params![
                    Uuid::new_v4().to_string(),
                    id,
                    path,
                    if index == 0 { 1 } else { 0 }
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())
    })();

    if let Err(error) = result {
        drop(conn);
        remove_unreferenced_image_files(&image_paths);
        return Err(error);
    }

    drop(conn);
    get_entry(id)
}

#[tauri::command]
fn update_entry(req: UpdateEntryRequest) -> Result<Entry, String> {
    let mut conn = DB.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    let new_image_paths = req.new_image_paths.clone();
    let mut removed_image_paths = Vec::new();

    let result = (|| -> Result<(), String> {
        validate_entry_fields(&req.name, &req.genre_id, &req.rating, &req.review, &conn)?;
        validate_external_links(&req.links)?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;

        for image_id in &req.removed_image_ids {
            if let Ok(path) = tx.query_row(
                "SELECT path FROM entry_images WHERE id = ?1 AND entry_id = ?2",
                params![image_id, req.id],
                |row| row.get::<_, String>(0),
            ) {
                removed_image_paths.push(path);
            }
        }

        tx.execute(
            "UPDATE entries SET name = ?1, genre_id = ?2, creator = ?3, rating = ?4, review = ?5,
             tasting_date = ?6, updated_at = ?7 WHERE id = ?8",
            params![
                req.name,
                req.genre_id,
                req.creator,
                req.rating,
                req.review,
                req.tasting_date,
                now,
                req.id
            ],
        )
        .map_err(|e| e.to_string())?;

        tx.execute(
            "DELETE FROM external_links WHERE entry_id = ?",
            params![req.id],
        )
        .map_err(|e| e.to_string())?;
        for link in &req.links {
            tx.execute(
                "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), req.id, link.url, link.label],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.execute("DELETE FROM tags WHERE entry_id = ?", params![req.id])
            .map_err(|e| e.to_string())?;
        for tag in &req.tags {
            tx.execute(
                "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
                params![Uuid::new_v4().to_string(), req.id, tag],
            )
            .map_err(|e| e.to_string())?;
        }

        for path in &new_image_paths {
            validate_project_image_path(path)?;
            let is_primary = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM entry_images WHERE entry_id = ?1 AND is_primary = 1)",
                    params![req.id],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?
                == 0;
            if is_primary {
                tx.execute(
                    "UPDATE entry_images SET is_primary = 0 WHERE entry_id = ?",
                    params![req.id],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.execute(
                "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
                params![Uuid::new_v4().to_string(), req.id, path, is_primary as i32],
            )
            .map_err(|e| e.to_string())?;
        }

        for image_id in &req.removed_image_ids {
            tx.execute(
                "DELETE FROM entry_images WHERE id = ?1 AND entry_id = ?2",
                params![image_id, req.id],
            )
            .map_err(|e| e.to_string())?;
        }
        ensure_primary_image(&tx, &req.id)?;

        tx.commit().map_err(|e| e.to_string())
    })();

    if let Err(error) = result {
        drop(conn);
        remove_unreferenced_image_files(&new_image_paths);
        return Err(error);
    }

    drop(conn);
    remove_unreferenced_image_files(&removed_image_paths);
    get_entry(req.id)
}

#[tauri::command]
fn delete_entries(ids: Vec<String>) -> Result<(), String> {
    let mut conn = DB.lock().map_err(|e| e.to_string())?;
    let mut paths = Vec::new();
    let result = (|| -> Result<(), String> {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        for id in &ids {
            let mut stmt = tx
                .prepare("SELECT path FROM entry_images WHERE entry_id = ?")
                .map_err(|e| e.to_string())?;
            paths.extend(
                stmt.query_map(params![id], |row| row.get::<_, String>(0))
                    .map_err(|e| e.to_string())?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())?,
            );
        }
        for id in &ids {
            tx.execute("DELETE FROM entries WHERE id = ?", params![id])
                .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    })();
    result?;
    drop(conn);
    remove_unreferenced_image_files(&paths);
    Ok(())
}

/// 按筛选条件统计条目数（与列表共用同一过滤逻辑）
#[tauri::command]
fn get_entries_count(query: Option<SearchQuery>) -> Result<i64, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let (where_sql, params_vec) = match &query {
        Some(q) => build_filter_sql(q),
        None => (String::new(), vec![]),
    };
    let sql = format!(
        "SELECT COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{}",
        where_sql
    );
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();
    let count: i64 = conn
        .query_row(&sql, params_refs.as_slice(), |row| row.get(0))
        .map_err(|e| e.to_string())?;
    Ok(count)
}

/// 所有条目的去重标签列表（用于筛选）
#[tauri::command]
fn get_all_tags() -> Result<Vec<String>, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT DISTINCT name FROM tags ORDER BY name")
        .map_err(|e| e.to_string())?;
    let tags = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(tags)
}

/// 所有条目的品鉴年份列表（降序，用于筛选）
#[tauri::command]
fn get_tasting_years() -> Result<Vec<i32>, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT CAST(strftime('%Y', tasting_date) AS INTEGER) AS y
             FROM entries
             WHERE tasting_date IS NOT NULL AND tasting_date != ''
             ORDER BY y DESC",
        )
        .map_err(|e| e.to_string())?;
    let years = stmt
        .query_map([], |row| row.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<i32>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(years)
}

// ============================================================================
// 图片管理命令
// ============================================================================

#[tauri::command]
fn add_entry_image(entry_id: String, path: String, is_primary: bool) -> Result<EntryImage, String> {
    let mut conn = DB.lock().map_err(|e| e.to_string())?;
    validate_project_image_path(&path)?;
    let id = Uuid::new_v4().to_string();

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    if is_primary {
        tx.execute(
            "UPDATE entry_images SET is_primary = 0 WHERE entry_id = ?",
            params![entry_id],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.execute(
        "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
        params![id, entry_id, path, is_primary as i32],
    )
    .map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;

    Ok(EntryImage {
        id,
        entry_id,
        path,
        is_primary,
    })
}

#[tauri::command]
fn delete_entry_image(id: String) -> Result<(), String> {
    use rusqlite::OptionalExtension;
    let mut conn = DB.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let image: Option<(String, String)> = tx
        .query_row(
            "SELECT path, entry_id FROM entry_images WHERE id = ?",
            params![id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    tx.execute("DELETE FROM entry_images WHERE id = ?", params![id])
        .map_err(|e| e.to_string())?;
    if let Some((_, entry_id)) = &image {
        ensure_primary_image(&tx, entry_id)?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    drop(conn);
    if let Some((path, _)) = image {
        remove_unreferenced_image_files(&[path]);
    }
    Ok(())
}

fn ensure_primary_image(conn: &Connection, entry_id: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE entry_images SET is_primary = 1 WHERE id = (
            SELECT id FROM entry_images WHERE entry_id = ?1 ORDER BY rowid LIMIT 1
        ) AND NOT EXISTS(SELECT 1 FROM entry_images WHERE entry_id = ?1 AND is_primary = 1)",
        params![entry_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn set_primary_image(id: String) -> Result<(), String> {
    let mut conn = DB.lock().map_err(|e| e.to_string())?;

    // 获取 entry_id
    let entry_id: String = conn
        .query_row(
            "SELECT entry_id FROM entry_images WHERE id = ?",
            params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;
    // 取消该条目所有主图
    tx.execute(
        "UPDATE entry_images SET is_primary = 0 WHERE entry_id = ?",
        params![entry_id],
    )
    .map_err(|e| e.to_string())?;

    // 设置新主图
    tx.execute(
        "UPDATE entry_images SET is_primary = 1 WHERE id = ?",
        params![id],
    )
    .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())
}

// ============================================================================
// 导出导入
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportEntry {
    pub name: String,
    pub genre_name: String,
    pub creator: Option<String>,
    pub rating: String,
    pub review: String,
    pub tasting_date: Option<String>,
    pub links: Vec<ExternalLink>,
    pub tags: Vec<String>,
    pub images: Vec<String>,
}

#[tauri::command]
fn export_entries(
    scope: String,
    format: String,
    include_images: bool,
    ids: Option<Vec<String>>,
    filter: Option<SearchQuery>,
) -> Result<String, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    let base_sql = "SELECT e.id, e.name, g.name, e.creator, e.rating, e.review, e.tasting_date, e.created_at, e.updated_at
         FROM entries e JOIN genres g ON e.genre_id = g.id";

    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = match scope.as_str() {
        "selected" => {
            let entry_ids = ids.unwrap_or_default();
            if entry_ids.is_empty() {
                return Err("未选择任何作品".to_string());
            }
            let placeholders: Vec<String> = entry_ids.iter().map(|_| "?".to_string()).collect();
            (
                format!("{} WHERE e.id IN ({})", base_sql, placeholders.join(",")),
                entry_ids
                    .into_iter()
                    .map(|s| Box::new(s) as Box<dyn rusqlite::ToSql>)
                    .collect(),
            )
        }
        "filtered" => {
            let (where_sql, params) = match &filter {
                Some(q) => build_filter_sql(q),
                None => (String::new(), vec![]),
            };
            (format!("{}{}", base_sql, where_sql), params)
        }
        _ => (base_sql.to_string(), vec![]),
    };

    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;

    type ExportRow = (
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        Option<String>,
        String,
        String,
    );
    let entries: Vec<ExportRow> = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok((
                row.get(0)?, // id
                row.get(1)?, // name
                row.get(2)?, // genre_name
                row.get(3)?, // creator
                row.get(4)?, // rating
                row.get(5)?, // review
                row.get(6)?, // tasting_date
                row.get(7)?, // created_at
                row.get(8)?, // updated_at
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(stmt);

    let mut export_entries = Vec::new();
    for (id, name, genre_name, creator, rating, review, tasting_date, _, _) in entries {
        let mut stmt = conn
            .prepare("SELECT id, entry_id, url, label FROM external_links WHERE entry_id = ?")
            .map_err(|e| e.to_string())?;
        let links: Vec<ExternalLink> = stmt
            .query_map(params![id], |row| {
                Ok(ExternalLink {
                    id: row.get(0)?,
                    entry_id: row.get(1)?,
                    url: row.get(2)?,
                    label: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        let mut stmt = conn
            .prepare("SELECT name FROM tags WHERE entry_id = ?")
            .map_err(|e| e.to_string())?;
        let tags: Vec<String> = stmt
            .query_map(params![id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);

        // HTML 格式始终包含图片；其余格式由 include_images 决定
        let with_images = include_images || format.as_str() == "html";
        let images: Vec<String> = if with_images {
            let mut stmt = conn
                .prepare("SELECT path FROM entry_images WHERE entry_id = ?")
                .map_err(|e| e.to_string())?;
            let result: Vec<String> = stmt
                .query_map(params![id], |row| row.get(0))
                .map_err(|e| e.to_string())?
                .filter_map(|r| r.ok())
                .collect();
            result
        } else {
            vec![]
        };

        export_entries.push(ExportEntry {
            name,
            genre_name,
            creator,
            rating,
            review,
            tasting_date,
            links,
            tags,
            images,
        });
    }

    let project_root = get_project_root();
    let export_root = project_root.join("exports");
    std::fs::create_dir_all(&export_root).map_err(|e| e.to_string())?;
    // 每次导出放入独立时间戳子目录，避免 cover_N.jpg 被下一次导出静默覆盖（损坏旧导出文件的图片引用）
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let export_dir = export_root.join(format!("export_{}_{}", timestamp, Uuid::new_v4()));
    std::fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    if include_images && matches!(format.as_str(), "json" | "csv") {
        for entry in &mut export_entries {
            for path in &mut entry.images {
                let bytes = read_registered_image(&conn, path)?;
                let name = format!("cover_{}.{}", Uuid::new_v4(), validate_image_bytes(&bytes)?);
                std::fs::write(export_dir.join(&name), bytes).map_err(|e| e.to_string())?;
                *path = name;
            }
        }
    }
    let content = match format.as_str() {
        "json" => serde_json::to_string_pretty(&export_entries).map_err(|e| e.to_string())?,
        "csv" => {
            let mut writer = csv::Writer::from_writer(Vec::new());
            writer
                .write_record([
                    "名称",
                    "类型",
                    "创作者",
                    "等级",
                    "评价",
                    "品鉴日期",
                    "标签",
                    "链接",
                    "图片",
                ])
                .map_err(|e| e.to_string())?;
            for entry in &export_entries {
                let links = entry
                    .links
                    .iter()
                    .map(|l| format!("{}:{}", l.label, l.url))
                    .collect::<Vec<_>>()
                    .join(";");
                let images = serde_json::to_string(&entry.images).map_err(|e| e.to_string())?;
                writer
                    .write_record([
                        entry.name.as_str(),
                        &entry.genre_name,
                        entry.creator.as_deref().unwrap_or(""),
                        &entry.rating,
                        &entry.review,
                        entry.tasting_date.as_deref().unwrap_or(""),
                        &entry.tags.join(";"),
                        &links,
                        &images,
                    ])
                    .map_err(|e| e.to_string())?;
            }
            String::from_utf8(writer.into_inner().map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
        }
        "markdown" => {
            let mut md = String::from("# 作品列表\n\n");
            // 勾选包含图片时：把图片复制到导出目录（与 .md 同目录），md 用相对路径引用。
            // 不使用 Base64 嵌入——图片多了会导致 .md 文件过大、编辑器无法正常显示。
            let mut cover_counter = 0u32;
            for entry in &export_entries {
                md.push_str(&format!("## {}\n\n", entry.name));
                if include_images {
                    for img_path in &entry.images {
                        if let Ok(bytes) = read_registered_image(&conn, img_path) {
                            cover_counter += 1;
                            let ext = validate_image_bytes(&bytes)?;
                            let fname = format!("cover_{}.{}", cover_counter, ext);
                            if std::fs::write(export_dir.join(&fname), &bytes).is_ok() {
                                md.push_str(&format!("![](./{})\n\n", fname));
                            }
                        }
                    }
                }
                md.push_str(&format!("- 类型：{}\n", entry.genre_name));
                md.push_str(&format!(
                    "- 创作者：{}\n",
                    entry.creator.as_deref().unwrap_or("")
                ));
                md.push_str(&format!("- 等级：**{}**\n", entry.rating));
                md.push_str(&format!(
                    "- 品鉴日期：{}\n",
                    entry.tasting_date.as_deref().unwrap_or("")
                ));
                if !entry.tags.is_empty() {
                    md.push_str(&format!("- 标签：{}\n", entry.tags.join("、")));
                }
                if !entry.links.is_empty() {
                    md.push_str("- 链接：\n");
                    for l in &entry.links {
                        md.push_str(&format!("  - [{}]({})\n", l.label, l.url));
                    }
                }
                md.push_str("\n### 评价\n\n");
                md.push_str(&entry.review);
                md.push_str("\n\n---\n\n");
            }
            md
        }
        "html" => {
            let mut html = String::from(
                "<!DOCTYPE html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"UTF-8\">\n\
                 <title>作品列表</title>\n<style>\n\
                 body { font-family: \"Microsoft YaHei\", sans-serif; max-width: 800px; margin: 0 auto; padding: 20px; }\n\
                 .entry { border-bottom: 1px solid #ddd; padding: 16px 0; }\n\
                 .meta { color: #666; }\n\
                 .rating { font-weight: bold; color: #c0392b; }\n\
                 .images img { max-width: 240px; max-height: 340px; margin: 4px; border-radius: 4px; }\n\
                 .review { white-space: pre-wrap; line-height: 1.6; }\n\
                 </style>\n</head>\n<body>\n<h1>作品列表</h1>\n",
            );
            for entry in &export_entries {
                html.push_str(&format!(
                    "<div class=\"entry\"><h2>{}</h2>\n",
                    escape_html(&entry.name)
                ));
                html.push_str(&format!(
                    "<p class=\"meta\">{} · <span class=\"rating\">{}</span> · {}{}</p>\n",
                    escape_html(&entry.genre_name),
                    escape_html(&entry.rating),
                    escape_html(entry.tasting_date.as_deref().unwrap_or("")),
                    entry
                        .creator
                        .as_deref()
                        .map(|c| format!(" · {}", escape_html(c)))
                        .unwrap_or_default()
                ));
                if !entry.tags.is_empty() {
                    html.push_str(&format!(
                        "<p class=\"meta\">标签：{}</p>\n",
                        entry
                            .tags
                            .iter()
                            .map(|t| escape_html(t))
                            .collect::<Vec<_>>()
                            .join("、")
                    ));
                }
                // 图片：Base64 嵌入
                html.push_str("<div class=\"images\">");
                for img_path in &entry.images {
                    if let Ok(bytes) = read_registered_image(&conn, img_path) {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let ext = validate_image_bytes(&bytes)?;
                        html.push_str(&format!(
                            "<img src=\"data:image/{};base64,{}\" alt=\"\">\n",
                            ext, b64
                        ));
                    }
                }
                html.push_str("</div>\n");
                html.push_str(&format!(
                    "<div class=\"review\">{}</div>\n</div>\n",
                    escape_html(&entry.review)
                ));
            }
            html.push_str("</body>\n</html>\n");
            html
        }
        "pdf" => return Err("PDF 导出未内置，请使用 Markdown（含图）自行转换".to_string()),
        _ => return Err("不支持的格式".to_string()),
    };

    // 保存文件（markdown 用 .md 后缀；文件位于本次导出的时间戳子目录内）
    let ext = if format.as_str() == "markdown" {
        "md"
    } else {
        format.as_str()
    };
    let export_path = export_dir.join(format!("export.{}", ext));
    std::fs::write(&export_path, &content).map_err(|e| e.to_string())?;

    Ok(export_path.to_string_lossy().to_string())
}

/// HTML 转义（导出 HTML 时防注入/防格式破坏）
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ============================================================================
// 数据库备份
// ============================================================================

// ============================================================================
// 封面爬取
// ============================================================================

/// 获取所有可用的封面数据源
#[tauri::command]
fn get_cover_sources() -> Vec<CoverSource> {
    vec![
        // 通用搜索引擎
        CoverSource {
            id: "bing_general".to_string(),
            name: "Bing 图片搜索（通用）".to_string(),
            source_type: "bing".to_string(),
            usage: "general".to_string(),
        },
        // 影视
        CoverSource {
            id: "douban_movie".to_string(),
            name: "豆瓣（影视）".to_string(),
            source_type: "douban".to_string(),
            usage: "movie".to_string(),
        },
        // 动漫
        CoverSource {
            id: "bangumi_anime".to_string(),
            name: "Bangumi（动漫）".to_string(),
            source_type: "bangumi".to_string(),
            usage: "anime".to_string(),
        },
        CoverSource {
            id: "anilist_anime".to_string(),
            name: "AniList（动漫）".to_string(),
            source_type: "anilist".to_string(),
            usage: "anime".to_string(),
        },
        // 图书
        CoverSource {
            id: "douban_book".to_string(),
            name: "豆瓣（图书）".to_string(),
            source_type: "douban".to_string(),
            usage: "book".to_string(),
        },
        // 音乐
        CoverSource {
            id: "itunes_music".to_string(),
            name: "iTunes（音乐）".to_string(),
            source_type: "itunes".to_string(),
            usage: "music".to_string(),
        },
        // 游戏
        CoverSource {
            id: "igdb_game".to_string(),
            name: "IGDB（游戏）".to_string(),
            source_type: "igdb".to_string(),
            usage: "game".to_string(),
        },
        CoverSource {
            id: "steam_game".to_string(),
            name: "Steam（游戏）".to_string(),
            source_type: "steam".to_string(),
            usage: "game".to_string(),
        },
    ]
}

/// 爬取封面候选图片
#[tauri::command]
fn fetch_cover_candidates(
    title: String,
    creator: Option<String>,
    source_id: String,
) -> Result<Vec<CoverCandidate>, String> {
    let client = build_client(15)?;

    match source_id.as_str() {
        "bing_general" => fetch_bing(&client, &title, creator.as_deref()),
        "douban_movie" => fetch_douban(&client, &title, "movie"),
        "douban_book" => fetch_douban(&client, &title, "book"),
        "bangumi_anime" => fetch_bangumi(&client, &title, creator.as_deref()),
        "anilist_anime" => fetch_anilist(&client, &title, creator.as_deref()),
        "itunes_music" => fetch_itunes(&client, &title, creator.as_deref()),
        "igdb_game" => fetch_igdb_search(&client, &title, creator.as_deref()),
        "steam_game" => fetch_steam_search(&client, &title, creator.as_deref()),
        _ => Err(format!("不支持的来源: {}", source_id)),
    }
}

// ---- 各数据源实现 ----

/// 构建 HTTP 客户端：支持通过 HTTP_PROXY/HTTPS_PROXY/ALL_PROXY 环境变量走代理（如 Clash），
/// 未设置代理时直连。超时秒数由调用方指定。
fn build_client(timeout_secs: u64) -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("重定向次数过多");
            }
            // 收到重定向响应才计时，保证服务端实际收到请求的间隔。
            if let Some(previous) = attempt.previous().last() {
                if let Ok(host) = request_host(previous.as_str()) {
                    match LAST_REQUEST_BY_HOST.lock() {
                        Ok(mut times) => { times.insert(host, Instant::now()); },
                        Err(error) => return attempt.error(std::io::Error::other(error.to_string())),
                    }
                }
            }
            match wait_for_request_slot(attempt.url().as_str()) {
                Ok(()) => attempt.follow(),
                Err(error) => attempt.error(std::io::Error::other(error)),
            }
        }))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(timeout_secs));

    let proxy_env = [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
        "all_proxy",
    ]
    .iter()
    .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()));
    if let Some(proxy) = proxy_env {
        if let Ok(p) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(p);
        }
    }

    builder
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
}

fn fetch_bing(
    client: &reqwest::blocking::Client,
    title: &str,
    creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    let query = match creator {
        Some(c) if !c.trim().is_empty() => format!("{} {} 封面", title, c),
        _ => format!("{} 封面", title),
    };
    let url = format!(
        "https://www.bing.com/images/async?q={}&first=1&count=20&relp=20",
        urlencoding::encode(&query)
    );

    let html = send_rate_limited(client.get(&url), &url)?
        .text()
        .map_err(|e| format!("读取失败: {}", e))?;

    let document = scraper::Html::parse_document(&html);
    let img_selector = scraper::Selector::parse("a.iusc").unwrap();

    let mut results = Vec::new();
    for el in document.select(&img_selector).take(15) {
        if let Some(m) = el.value().attr("m") {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(m) {
                if let Some(img_url) = json.get("murl").and_then(|v| v.as_str()) {
                    let thumb = json
                        .get("turl")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    results.push(CoverCandidate {
                        url: img_url.to_string(),
                        thumbnail_url: thumb,
                        title: json
                            .get("desc")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        source: "bing_general".to_string(),
                        width: None,
                        height: None,
                    });
                }
            }
        }
    }
    Ok(results)
}

fn fetch_douban(
    client: &reqwest::blocking::Client,
    title: &str,
    cat: &str,
) -> Result<Vec<CoverCandidate>, String> {
    let url = format!(
        "https://search.douban.com/{}/subject_search?search_text={}&cat={}",
        cat,
        urlencoding::encode(title),
        cat
    );

    let resp = send_rate_limited(
        client
            .get(&url)
            .header("Referer", format!("https://{}.douban.com/", cat)),
        &url,
    )?;

    let text = resp.text().map_err(|e| format!("读取失败: {}", e))?;

    // 新版搜索页把结果嵌入 window.__DATA__ = {...}; JSON 中
    let start = text
        .find("window.__DATA__")
        .ok_or_else(|| "未找到搜索结果".to_string())?;
    let eq = text[start..]
        .find('=')
        .map(|i| start + i + 1)
        .unwrap_or(start);
    // JSON 结束位置：优先找 };，其次找 </script>，取较早者
    let semi = text[eq..].find("};").map(|i| eq + i + 1);
    let script = text[eq..].find("</script>").map(|i| eq + i);
    let json_end = match (semi, script) {
        (Some(a), Some(b)) => a.min(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => text.len(),
    };

    let json: serde_json::Value = serde_json::from_str(&text[eq..json_end])
        .map_err(|e| format!("解析搜索结果失败: {}", e))?;

    let mut results = Vec::new();
    if let Some(items) = json.get("items").and_then(|v| v.as_array()) {
        for item in items {
            // 跳过 "搜索更多 xx" 之类的占位项
            if item.get("tpl_name").and_then(|v| v.as_str()) == Some("search_more") {
                continue;
            }
            if let Some(cover) = item.get("cover_url").and_then(|v| v.as_str()) {
                // 小图尺寸替换为原图（movie: s_ratio_poster / book: m）
                let hi = cover
                    .replace("/s_ratio_poster/", "/l/")
                    .replace("/subject/m/", "/subject/l/");
                results.push(CoverCandidate {
                    url: hi,
                    thumbnail_url: Some(cover.to_string()),
                    title: item
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    source: format!("douban_{}", cat),
                    width: None,
                    height: None,
                });
            }
        }
    }
    Ok(results)
}

/// 读取 Bangumi Cookie 配置文件（config/bangumi_cookie.txt），未配置返回 None
fn load_bangumi_cookie() -> Option<String> {
    let path = get_project_root().join("config").join("bangumi_cookie.txt");
    read_cookie_file(&path)
}

/// 从文件读取 cookie：去除首尾空白与换行，容忍 "Cookie:" 前缀，内容为空返回 None
fn read_cookie_file(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let cleaned = content.replace("\r", "").replace("\n", "");
    let trimmed = cleaned.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    // 容错：从 Network 面板复制的整行可能带 "Cookie: " 前缀
    let stripped = trimmed
        .strip_prefix("Cookie:")
        .or_else(|| trimmed.strip_prefix("cookie:"))
        .map(|s| s.trim().to_string())
        .unwrap_or(trimmed);
    if stripped.is_empty() {
        None
    } else {
        Some(stripped)
    }
}

fn fetch_bangumi(
    client: &reqwest::blocking::Client,
    title: &str,
    _creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    let cookie = load_bangumi_cookie();

    match search_bangumi_web(client, title, cookie.as_deref()) {
        Ok(list) => Ok(list),
        Err(e) if cookie.is_some() => {
            // Cookie 请求失败：回退为匿名搜索（不显示 R18）
            eprintln!("[WARN] Bangumi cookie 请求失败，回退匿名搜索: {}", e);
            search_bangumi_web(client, title, None)
        }
        Err(e) => Err(e),
    }
}

/// 网页端 Bangumi 搜索（bgm.tv/subject_search，cookie 生效）：配置了 cookie 时携带，
/// 账号具备 R18 访问权限则 R18 条目可见
fn search_bangumi_web(
    client: &reqwest::blocking::Client,
    title: &str,
    cookie: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    let url = format!(
        "https://bgm.tv/subject_search/{}?cat=2",
        urlencoding::encode(title)
    );

    let mut req = client.get(&url);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }

    let html = send_rate_limited(req, &url)?
        .text()
        .map_err(|e| format!("读取失败: {}", e))?;

    let document = scraper::Html::parse_document(&html);
    let li_sel = scraper::Selector::parse("#browserItemList li").unwrap();
    let img_sel = scraper::Selector::parse("img.cover").unwrap();
    let title_sel = scraper::Selector::parse("a[title]").unwrap();

    let mut results = Vec::new();
    for li in document.select(&li_sel).take(15) {
        // 封面缩略图
        let thumb = li.select(&img_sel).next().and_then(|img| {
            img.value().attr("src").map(|src| {
                if src.starts_with("//") {
                    format!("https:{}", src)
                } else {
                    src.to_string()
                }
            })
        });
        // 标题：li 内第一个指向条目的带 title 链接
        let name = li
            .select(&title_sel)
            .find(|a| {
                a.value()
                    .attr("href")
                    .map(|h| h.starts_with("/subject/"))
                    .unwrap_or(false)
            })
            .and_then(|a| a.value().attr("title").map(|t| t.to_string()));

        if let Some(thumb_url) = thumb {
            results.push(CoverCandidate {
                url: upgrade_cover_url(&thumb_url),
                thumbnail_url: Some(thumb_url),
                title: name,
                source: "bangumi_anime".to_string(),
                width: None,
                height: None,
            });
        }
    }
    Ok(results)
}

/// lain.bgm.tv 缩略图 URL 转原图：/r/<size>/pic/ → /pic/
fn upgrade_cover_url(src: &str) -> String {
    if let Some(r_pos) = src.find("/r/") {
        let after = &src[r_pos + 3..];
        if let Some(rel) = after.find('/') {
            let pic_pos = r_pos + 3 + rel;
            if src[pic_pos..].starts_with("/pic/") {
                let mut out = String::with_capacity(src.len());
                out.push_str(&src[..r_pos]);
                out.push_str(&src[pic_pos..]);
                return out;
            }
        }
    }
    src.to_string()
}

fn fetch_anilist(
    client: &reqwest::blocking::Client,
    title: &str,
    _creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    let query = r#"
        query ($search: String) {
            Page(perPage: 15) {
                media(search: $search, type: ANIME) {
                    title { romaji english }
                    coverImage { large medium }
                }
            }
        }
    "#;
    let body = serde_json::json!({
        "query": query,
        "variables": { "search": title }
    });

    let resp = send_rate_limited(
        client.post("https://graphql.anilist.co").json(&body),
        "https://graphql.anilist.co",
    )?;

    let json: serde_json::Value = resp.json().map_err(|e| format!("解析失败: {}", e))?;
    let mut results = Vec::new();
    if let Some(media) = json
        .get("data")
        .and_then(|d| d.get("Page"))
        .and_then(|p| p.get("media"))
        .and_then(|m| m.as_array())
    {
        for item in media {
            let cover = item
                .get("coverImage")
                .and_then(|c| c.get("large"))
                .and_then(|v| v.as_str());
            let thumb = item
                .get("coverImage")
                .and_then(|c| c.get("medium"))
                .and_then(|v| v.as_str());
            if let Some(img) = cover {
                results.push(CoverCandidate {
                    url: img.to_string(),
                    thumbnail_url: thumb.map(|s| s.to_string()),
                    title: None,
                    source: "anilist_anime".to_string(),
                    width: None,
                    height: None,
                });
            }
        }
    }
    Ok(results)
}

fn fetch_itunes(
    client: &reqwest::blocking::Client,
    title: &str,
    creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    let mut query = title.to_string();
    if let Some(c) = creator {
        query.push(' ');
        query.push_str(c);
    }
    let url = format!(
        "https://itunes.apple.com/search?term={}&media=music&entity=album&limit=15",
        urlencoding::encode(&query)
    );

    let resp = send_rate_limited(client.get(&url), &url)?;
    let json: serde_json::Value = resp.json().map_err(|e| format!("解析失败: {}", e))?;

    let mut results = Vec::new();
    if let Some(list) = json.get("results").and_then(|r| r.as_array()) {
        for item in list {
            if let Some(art) = item.get("artworkUrl100").and_then(|v| v.as_str()) {
                // 100x100 -> 600x600
                let hi = art.replace("100x100bb", "600x600bb");
                results.push(CoverCandidate {
                    url: hi.clone(),
                    thumbnail_url: Some(art.to_string()),
                    title: item
                        .get("collectionName")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    source: "itunes_music".to_string(),
                    width: None,
                    height: None,
                });
            }
        }
    }
    Ok(results)
}

fn fetch_igdb_search(
    client: &reqwest::blocking::Client,
    title: &str,
    _creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    // IGDB 需要 API key, 使用 Steam 商店搜索替代
    fetch_steam_search(client, title, _creator)
}

fn fetch_steam_search(
    client: &reqwest::blocking::Client,
    title: &str,
    _creator: Option<&str>,
) -> Result<Vec<CoverCandidate>, String> {
    // Steam 搜索 API
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&cc=cn&l=schinese",
        urlencoding::encode(title)
    );
    let resp = send_rate_limited(client.get(&url), &url)?;
    let json: serde_json::Value = resp.json().map_err(|e| format!("解析失败: {}", e))?;

    let mut results = Vec::new();
    if let Some(list) = json.get("items").and_then(|i| i.as_array()) {
        for item in list {
            let tiny = item
                .get("tiny_image")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            if let Some(t) = &tiny {
                results.push(CoverCandidate {
                    url: t.clone(),
                    thumbnail_url: tiny.clone(),
                    title: item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    source: "steam_game".to_string(),
                    width: None,
                    height: None,
                });
            }
        }
    }
    Ok(results)
}

// ---- 封面下载 ----

fn fetch_cover_preview_data(url: &str) -> Result<String, String> {
    let client = build_client(30)?;
    let (bytes, ext) = validate_image_response(send_rate_limited(client.get(url), url)?)?;
    let mime = if ext == "jpg" { "jpeg" } else { ext };
    Ok(format!(
        "data:image/{};base64,{}",
        mime,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

#[tauri::command]
async fn fetch_cover_preview(url: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_cover_preview_data(&url))
        .await
        .map_err(|e| e.to_string())?
}

/// 下载封面图片到本地项目 resource/cover_image 目录
#[tauri::command]
fn download_cover(url: String, title: String, creator: Option<String>) -> Result<String, String> {
    let client = build_client(30)?;
    let resp = send_rate_limited(client.get(&url), &url)?;
    let (bytes, _) = validate_image_response(resp)?;
    store_image_bytes(
        &bytes,
        &format!("{}_{}", title, creator.unwrap_or_default()),
    )
}

/// 复制用户选择的本地图片，扩展名取自内容，文件名使用 UUID 防止并发覆盖。
#[tauri::command]
fn import_local_image(
    source_path: String,
    title: String,
    creator: Option<String>,
) -> Result<String, String> {
    let bytes = read_image_file(std::path::Path::new(&source_path))?;
    store_image_bytes(
        &bytes,
        &format!("{}_{}", title, creator.unwrap_or_default()),
    )
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn backup_connection_to_path(
    source: &Connection,
    backup_path: &std::path::Path,
) -> Result<(), String> {
    let mut destination = Connection::open(backup_path).map_err(|e| e.to_string())?;
    let backup = rusqlite::backup::Backup::new(source, &mut destination)
        .map_err(|e| format!("创建备份会话失败: {}", e))?;
    backup
        .run_to_completion(5, std::time::Duration::from_millis(250), None)
        .map_err(|e| format!("备份失败: {}", e))
}

#[tauri::command]
fn backup_database() -> Result<String, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    retry_image_cleanup(&conn, &[])?;
    let project_root = get_project_root();
    let backup_dir = project_root.join("backups");
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S_%3f");
    let backup_path = backup_dir.join(format!("backup_{}_{}.db", timestamp, Uuid::new_v4()));
    if let Err(error) = backup_connection_to_path(&conn, &backup_path) {
        let _ = std::fs::remove_file(&backup_path);
        return Err(error);
    }
    drop(conn);

    // 清理旧备份：只保留最近 20 份（含关闭时自动备份，防止无限累积）
    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        let mut backups: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".db"))
            .collect();
        backups.sort_by_key(|e| e.file_name());
        if backups.len() > 20 {
            for old in backups.iter().take(backups.len() - 20) {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }

    Ok(backup_path.to_string_lossy().to_string())
}

/// 核心：把源 SQLite 文件在线导入目标连接（校验文件头 → Backup API）
fn import_db_into(source_path: &str, dst: &mut Connection) -> Result<(), String> {
    // 校验是 SQLite 文件（magic header）
    {
        use std::io::Read;
        let mut f = std::fs::File::open(source_path).map_err(|e| format!("无法打开文件: {}", e))?;
        let mut header = [0u8; 16];
        f.read_exact(&mut header)
            .map_err(|e| format!("读取文件失败: {}", e))?;
        if &header != b"SQLite format 3\0" {
            return Err("所选文件不是有效的 SQLite 数据库".to_string());
        }
    }

    // 在线备份 API：把源文件数据导入目标连接（无需关闭连接/覆盖文件）
    let src = Connection::open_with_flags(source_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("无法打开源数据库: {}", e))?;
    let backup =
        rusqlite::backup::Backup::new(&src, dst).map_err(|e| format!("创建导入会话失败: {}", e))?;
    backup
        .run_to_completion(5, std::time::Duration::from_millis(250), None)
        .map_err(|e| format!("导入失败: {}", e))?;

    Ok(())
}

/// 导入数据库（恢复备份）：先自动备份当前库，再在线导入所选文件
#[tauri::command]
fn import_database(source_path: String) -> Result<(), String> {
    // 覆盖前自动备份当前数据库
    backup_database().map_err(|e| format!("备份当前数据库失败: {}", e))?;

    let mut dst = DB.lock().map_err(|e| e.to_string())?;
    import_db_into(&source_path, &mut dst)?;
    init_database(&dst).map_err(|e| e.to_string())
}

/// 统计面板数据
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total: i64,
    pub rating_dist: Vec<(String, i64)>,
    pub genre_dist: Vec<(String, i64)>,
    pub year_dist: Vec<(String, i64)>,
}

#[tauri::command]
fn get_stats(query: Option<SearchQuery>) -> Result<Stats, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let (where_sql, params_vec) = match &query {
        Some(q) => build_filter_sql(q),
        None => (String::new(), vec![]),
    };
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|b| b.as_ref()).collect();

    let total: i64 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{}",
                where_sql
            ),
            params_refs.as_slice(),
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    let rating_dist: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT e.rating, COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{} GROUP BY e.rating ORDER BY CASE e.rating WHEN 'S' THEN 0 WHEN 'A' THEN 1 WHEN 'B' THEN 2 ELSE 3 END",
                where_sql
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    let genre_dist: Vec<(String, i64)> = {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT g.name, COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{} GROUP BY g.name ORDER BY COUNT(DISTINCT e.id) DESC",
                where_sql
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    let year_dist: Vec<(String, i64)> = {
        // 品鉴日期过滤条件与筛选条件合并（避免出现两个 WHERE）
        let year_where = if where_sql.is_empty() {
            " WHERE e.tasting_date IS NOT NULL AND e.tasting_date != ''".to_string()
        } else {
            format!(
                "{} AND e.tasting_date IS NOT NULL AND e.tasting_date != ''",
                where_sql
            )
        };
        let mut stmt = conn
            .prepare(&format!(
                "SELECT strftime('%Y', e.tasting_date), COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{} GROUP BY 1 ORDER BY 1",
                year_where
            ))
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows
    };

    Ok(Stats {
        total,
        rating_dist,
        genre_dist,
        year_dist,
    })
}

/// 按名称查找类型，不存在则创建自定义类型
fn find_or_create_genre(conn: &Connection, name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("类型名为空".to_string());
    }
    match conn.query_row(
        "SELECT id FROM genres WHERE name = ?1",
        params![name],
        |row| row.get(0),
    ) {
        Ok(id) => return Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => {}
        Err(error) => return Err(error.to_string()),
    }
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO genres (id, name, is_default, created_at) VALUES (?1, ?2, 0, ?3)",
        params![id, name, Utc::now().to_rfc3339()],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// 解析 "标签:URL;标签2:URL2" 形式的链接字符串（导出 CSV 的链接列格式）
fn parse_links(s: &str) -> Vec<ExternalLink> {
    s.split(';')
        .filter(|x| !x.trim().is_empty())
        .filter_map(|pair| {
            if let Some(pos) = pair.find("http") {
                let label = pair[..pos].trim_end_matches(':').trim().to_string();
                let url = pair[pos..].trim().to_string();
                if url.is_empty() {
                    None
                } else {
                    Some(ExternalLink {
                        id: String::new(),
                        entry_id: String::new(),
                        url,
                        label,
                    })
                }
            } else {
                None
            }
        })
        .collect()
}

/// 解析导入 CSV（列序与导出一致：名称,类型,创作者,等级,评价,品鉴日期,标签,链接）
fn parse_import_csv(content: &str) -> Result<Vec<ExportEntry>, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_reader(content.as_bytes());
    let image_column = rdr
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .position(|h| h == "图片" || h == "images");
    let mut out = Vec::new();
    for record in rdr.records() {
        let record = record.map_err(|e| e.to_string())?;
        let name = record.get(0).unwrap_or("").trim().to_string();
        let genre = record.get(1).unwrap_or("").trim().to_string();
        let creator_raw = record.get(2).unwrap_or("").trim();
        let creator = if creator_raw.is_empty() {
            None
        } else {
            Some(creator_raw.to_string())
        };
        let rating = record.get(3).unwrap_or("").trim().to_string();
        let review = record.get(4).unwrap_or("").to_string();
        let date_raw = record.get(5).unwrap_or("").trim();
        let tasting_date = if date_raw.is_empty() {
            None
        } else {
            Some(date_raw.to_string())
        };
        let tags: Vec<String> = record
            .get(6)
            .unwrap_or("")
            .split(';')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let links = parse_links(record.get(7).unwrap_or(""));
        let images = match image_column
            .and_then(|i| record.get(i))
            .filter(|s| !s.trim().is_empty())
        {
            Some(value) => serde_json::from_str::<Vec<String>>(value)
                .map_err(|e| format!("图片列必须是路径 JSON 数组: {}", e))?,
            None => vec![],
        };
        out.push(ExportEntry {
            name,
            genre_name: genre,
            creator,
            rating,
            review,
            tasting_date,
            links,
            tags,
            images,
        });
    }
    Ok(out)
}

/// 导入 JSON/CSV 文件（批量新增条目，类型缺失自动创建）
#[tauri::command]
fn import_entries(path: String, format: String) -> Result<ImportResult, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    let import_file = std::path::PathBuf::from(&path);
    let mut conn = DB.lock().map_err(|e| e.to_string())?;

    let records: Vec<ExportEntry> = match format.as_str() {
        "json" => serde_json::from_str::<Vec<ExportEntry>>(&content)
            .map_err(|e| format!("JSON 解析失败: {}", e))?,
        "csv" => parse_import_csv(&content)?,
        _ => return Err("不支持的格式".to_string()),
    };

    let mut imported = 0usize;
    let mut failed = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for rec in records {
        let record_name = rec.name.clone();
        let (stored_images, copied_files) = match copy_imported_images(&rec.images, &import_file) {
            Ok(value) => value,
            Err(error) => {
                failed += 1;
                if errors.len() < 20 {
                    errors.push(format!("《{}》: {}", record_name, error));
                }
                continue;
            }
        };

        let result = (|| -> Result<(), String> {
            validate_external_links(&rec.links)?;
            let tx = conn.transaction().map_err(|e| e.to_string())?;
            let genre_id = find_or_create_genre(&tx, &rec.genre_name)?;
            validate_entry_fields(&rec.name, &genre_id, &rec.rating, &rec.review, &tx)?;

            let id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            tx.execute(
                "INSERT INTO entries (id, name, genre_id, creator, rating, review, tasting_date, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    rec.name,
                    genre_id,
                    rec.creator,
                    rec.rating,
                    rec.review,
                    rec.tasting_date,
                    now,
                    now
                ],
            )
            .map_err(|e| e.to_string())?;

            for link in &rec.links {
                if link.url.trim().is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
                    params![Uuid::new_v4().to_string(), id, link.url, link.label],
                )
                .map_err(|e| e.to_string())?;
            }
            for tag in &rec.tags {
                if tag.trim().is_empty() {
                    continue;
                }
                tx.execute(
                    "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
                    params![Uuid::new_v4().to_string(), id, tag],
                )
                .map_err(|e| e.to_string())?;
            }
            for (i, image_path) in stored_images.iter().enumerate() {
                tx.execute(
                    "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
                    params![Uuid::new_v4().to_string(), id, image_path, if i == 0 { 1 } else { 0 }],
                )
                .map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        })();

        match result {
            Ok(()) => imported += 1,
            Err(error) => {
                remove_copied_files(&copied_files);
                failed += 1;
                if errors.len() < 20 {
                    errors.push(format!("《{}》: {}", record_name, error));
                }
            }
        }
    }

    Ok(ImportResult {
        imported,
        failed,
        errors,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

fn read_registered_image(conn: &Connection, path: &str) -> Result<Vec<u8>, String> {
    let resolved = if let Ok(managed) = validate_project_image_path(path) {
        managed
    } else {
        let raw = std::path::Path::new(path);
        if !raw.is_absolute() {
            return Err("图片路径越出封面目录".to_string());
        }
        let registered: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM entry_images WHERE path = ?1)",
                params![path],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if !registered {
            return Err("只能读取已登记的旧图片".to_string());
        }
        raw.to_path_buf()
    };
    read_image_file(&resolved)
}

#[tauri::command]
fn get_image_base64(path: String) -> Result<String, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    let bytes = read_registered_image(&conn, &path)?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// 保存前端生成的图片（分享卡片等）：data URL → 文件
#[tauri::command]
fn save_base64_image(data_url: String, path: String) -> Result<(), String> {
    let target = std::path::Path::new(&path);
    if !target.is_absolute()
        || !target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return Err("分享卡片只能保存为绝对路径的 PNG 文件".to_string());
    }
    let b64 = data_url
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| "分享卡片必须是 PNG 图片".to_string())?;
    if b64.len() > 14 * 1024 * 1024 {
        return Err("图片超过 10 MiB 大小限制".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("图片数据解码失败: {}", e))?;
    if validate_image_bytes(&bytes)? != "png" {
        return Err("分享卡片必须是 PNG 图片".into());
    }
    std::fs::write(&path, &bytes).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(())
}

// ============================================================================
// 程序入口
// ============================================================================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 初始化数据库
    {
        let _lock = DB.lock().expect("Failed to acquire database lock");
        // 锁立即释放，确保其他命令可以正常获取
    }

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            // 类型管理
            get_genres,
            create_genre,
            delete_genre,
            // 条目管理
            get_entries,
            get_entry,
            create_entry,
            update_entry,
            delete_entries,
            get_entries_count,
            get_all_tags,
            get_tasting_years,
            get_stats,
            // 图片管理
            add_entry_image,
            delete_entry_image,
            set_primary_image,
            // 导出导入
            export_entries,
            import_entries,
            // 备份
            backup_database,
            import_database,
            // 封面爬取
            get_cover_sources,
            fetch_cover_candidates,
            fetch_cover_preview,
            download_cover,
            import_local_image,
            get_image_base64,
            save_base64_image,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            // 关闭时自动备份数据库
            if let Err(e) = backup_database() {
                eprintln!("[WARN] 退出时自动备份失败: {}", e);
            }
        }
    });
}

// ============================================================================
// 封面数据源网络测试（cargo test -- --nocapture）
// ============================================================================

#[cfg(test)]
mod reliability_tests;

#[cfg(test)]
mod cover_tests {
    use super::*;

    #[test]
    fn test_all_cover_sources() {
        let cases = [
            ("bing_general", "尼尔：自动人形", None),
            ("douban_movie", "秒速五厘米", Some("新海诚")),
            ("douban_book", "人间失格", Some("太宰治")),
            ("bangumi_anime", "葬送的芙莉莲", None),
            ("anilist_anime", "Frieren", None),
            ("itunes_music", "ヨルシカ", Some("Yorushika")),
            ("steam_game", "Monster Hunter", None),
            ("igdb_game", "Monster Hunter Wilds", None),
        ];

        let mut failures: Vec<String> = Vec::new();
        for (src, title, creator) in cases {
            match fetch_cover_candidates(
                title.to_string(),
                creator.map(|c| c.to_string()),
                src.to_string(),
            ) {
                Ok(list) => {
                    println!("[OK] {} ({}): {} candidates", src, title, list.len());
                    if let Some(first) = list.first() {
                        println!("     first url: {}", first.url);
                    }
                    if list.is_empty() {
                        failures.push(format!("{} 返回 0 个候选", src));
                    }
                }
                Err(e) => {
                    println!("[FAIL] {} ({}): {}", src, title, e);
                    failures.push(format!("{}: {}", src, e));
                }
            }
        }

        assert!(failures.is_empty(), "失败源: {:?}", failures);
    }

    #[test]
    fn test_foreign_keys_are_enabled_and_cascade() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF").unwrap();
        init_database(&conn).unwrap();

        let foreign_keys: i64 = conn
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);

        let genre_id: String = conn
            .query_row("SELECT id FROM genres LIMIT 1", [], |row| row.get(0))
            .unwrap();
        conn.execute(
            "INSERT INTO entries (id, name, genre_id, rating, review, created_at, updated_at)
             VALUES ('entry', '作品', ?1, 'S', '这是一段足够长的评价文本', 'now', 'now')",
            params![genre_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entry_images (id, entry_id, path, is_primary)
             VALUES ('image', 'entry', 'resource/cover_image/a.jpg', 1)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM entries WHERE id = 'entry'", [])
            .unwrap();
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_images WHERE entry_id = 'entry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn test_read_cookie_file() {
        let dir = std::env::temp_dir().join(format!("prefdb_cookie_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cookie.txt");

        // 文件不存在 → None
        assert!(read_cookie_file(&path).is_none());

        // 空内容 → None
        std::fs::write(&path, "  \n\t\n").unwrap();
        assert!(read_cookie_file(&path).is_none());

        // 多行 + 换行/回车 → 合并为单行
        std::fs::write(&path, "chii_auth=abc123;\n chii_sid=xyz;\r\n").unwrap();
        let v = read_cookie_file(&path).unwrap();
        assert_eq!(v, "chii_auth=abc123; chii_sid=xyz;");

        // 带 "Cookie: " 前缀 → 剥离
        std::fs::write(&path, "Cookie: chii_auth=abc123; chii_sid=xyz;").unwrap();
        let v = read_cookie_file(&path).unwrap();
        assert_eq!(v, "chii_auth=abc123; chii_sid=xyz;");

        // 只有 "Cookie:" 前缀 → None
        std::fs::write(&path, "Cookie:").unwrap();
        assert!(read_cookie_file(&path).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_upgrade_cover_url() {
        // 标准缩略图 → 原图
        assert_eq!(
            upgrade_cover_url("https://lain.bgm.tv/r/400/pic/cover/l/13/c5/400602_ZI8Y9.jpg"),
            "https://lain.bgm.tv/pic/cover/l/13/c5/400602_ZI8Y9.jpg"
        );
        // 非 lain 图床 / 无尺寸段 → 原样返回
        assert_eq!(
            upgrade_cover_url(
                "https://s4.anilist.co/file/anilistcdn/media/anime/cover/medium/bx1.jpg"
            ),
            "https://s4.anilist.co/file/anilistcdn/media/anime/cover/medium/bx1.jpg"
        );
        // 无协议前缀
        assert_eq!(
            upgrade_cover_url("//lain.bgm.tv/r/100/pic/cover/l/13/c5/x.jpg"),
            "//lain.bgm.tv/pic/cover/l/13/c5/x.jpg"
        );
    }

    #[test]
    fn test_image_path_resolution() {
        // 相对路径 → 拼项目根（绝对路径且以相对路径结尾）
        let rel = resolve_image_path("resource/cover_image/a.jpg");
        assert!(rel.is_absolute());
        assert!(rel
            .to_string_lossy()
            .ends_with("resource/cover_image/a.jpg"));

        // 绝对路径（旧数据）→ 原样返回
        let abs = resolve_image_path("D:\\some\\where\\b.jpg");
        assert_eq!(abs.to_string_lossy(), "D:\\some\\where\\b.jpg");

        // 相对 → 绝对 → 相对 往返
        let back = to_project_rel_path(&rel);
        assert_eq!(back, "resource/cover_image/a.jpg");
    }

    #[test]
    fn test_import_db() {
        let dir = std::env::temp_dir().join(format!("prefdb_import_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src_path = dir.join("src.db");
        let dst_path = dir.join("dst.db");

        // 源库：建表 + 插数据
        {
            let src = Connection::open(&src_path).unwrap();
            src.execute_batch("CREATE TABLE t(x INTEGER); INSERT INTO t VALUES (42);")
                .unwrap();
        }
        // 目标库：空连接
        let mut dst = Connection::open(&dst_path).unwrap();

        // 导入后数据可用
        import_db_into(src_path.to_str().unwrap(), &mut dst).unwrap();
        let v: i64 = dst.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(v, 42);

        // 非法文件被拒绝
        let bad = dir.join("bad.txt");
        std::fs::write(&bad, "not a sqlite db").unwrap();
        assert!(import_db_into(bad.to_str().unwrap(), &mut dst).is_err());

        drop(dst);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_parse_import_csv() {
        let csv = "名称,类型,创作者,等级,评价,品鉴日期,标签,链接\n\
测试作品,游戏,某人,S,这是一段超过十个字符的评价,2026-01-01,科幻;治愈,豆瓣:https://douban.com/x\n\
坏条目,游戏,,X,太短,,,\n\
\"作品A\",音乐,艺术家,S,\"评价，带中文逗号和\"\"引号\"\"足够长足够长\",2026-02-01,标签1,\n";
        let recs = parse_import_csv(csv).unwrap();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0].name, "测试作品");
        assert_eq!(recs[0].genre_name, "游戏");
        assert_eq!(recs[0].tags, vec!["科幻", "治愈"]);
        assert_eq!(recs[0].links.len(), 1);
        assert_eq!(recs[0].links[0].label, "豆瓣");
        assert_eq!(recs[0].links[0].url, "https://douban.com/x");
        // 引号转义解析
        assert_eq!(recs[2].name, "作品A");
        assert_eq!(recs[2].review, "评价，带中文逗号和\"引号\"足够长足够长");
    }

    #[test]
    fn test_find_or_create_genre() {
        let dir = std::env::temp_dir().join(format!("prefdb_genre_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("g.db");
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE genres (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, is_default INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL);",
        )
        .unwrap();

        // 不存在 → 创建
        let id1 = find_or_create_genre(&conn, "自定义类型").unwrap();
        // 再次 → 复用同一 id
        let id2 = find_or_create_genre(&conn, " 自定义类型 ").unwrap();
        assert_eq!(id1, id2);
        // 空名 → 错误
        assert!(find_or_create_genre(&conn, "  ").is_err());

        drop(conn);
        std::fs::remove_dir_all(&dir).ok();
    }
}
