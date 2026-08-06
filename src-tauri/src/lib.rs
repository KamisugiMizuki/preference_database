use base64::Engine;
use chrono::Utc;
use once_cell::sync::Lazy;
use rusqlite::{params, Connection, Result as SqliteResult};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub keyword: Option<String>,
    pub search_field: Option<String>, // "all", "name", "tags", "review"
    pub genre_ids: Vec<String>,
    pub ratings: Vec<String>,
    pub tag_filter: Vec<String>,
    pub year: Option<i32>,
    pub sort_by: String,       // "name", "rating", "tasting_date", "updated_at"
    pub sort_order: String,     // "asc", "desc"
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

fn get_project_root() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(root) = exe.ancestors().nth(4) {
            return root.to_path_buf();
        }
    }
    std::path::PathBuf::from(".")
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

/// 绝对路径转项目相对路径（用于入库）；不在项目内则原样返回
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
    Mutex::new(conn)
});

fn init_database(conn: &Connection) -> SqliteResult<()> {
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

    // 创建索引
    conn.execute("CREATE INDEX IF NOT EXISTS idx_entries_genre ON entries(genre_id)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_entries_rating ON entries(rating)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_entries_name ON entries(name)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tags_entry ON tags(entry_id)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_tags_name ON tags(name)", [])?;
    conn.execute("CREATE INDEX IF NOT EXISTS idx_links_entry ON external_links(entry_id)", [])?;

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
        return Err(format!("作品名称长度不能超过 200 字符（当前 {}）", name_len));
    }
    let review_len = review.trim().chars().count();
    if review_len < 10 {
        return Err(format!("个人评价文段至少需要 10 字符（当前 {}）", review_len));
    }
    if review_len > 20000 {
        return Err(format!("个人评价文段不能超过 20000 字符（当前 {}）", review_len));
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

    conn.execute("DELETE FROM genres WHERE id = ?1 AND is_default = 0", params![id])
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

    // 排序
    let sort_col = match query.sort_by.as_str() {
        "name" => "e.name",
        "rating" => "e.rating",
        "tasting_date" => "e.tasting_date",
        _ => "e.updated_at",
    };
    let sort_dir = if query.sort_order == "asc" { "ASC" } else { "DESC" };
    sql.push_str(&format!(" ORDER BY {} {}", sort_col, sort_dir));

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
        
        if primary_image.is_some() {
            eprintln!("[DEBUG] Found primary_image for entry {}: {}", entry.name, primary_image.as_ref().unwrap());
        }

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
    let conn = DB.lock().map_err(|e| e.to_string())?;
    validate_entry_fields(&req.name, &req.genre_id, &req.rating, &req.review, &conn)?;
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO entries (id, name, genre_id, creator, rating, review, tasting_date, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, req.name, req.genre_id, req.creator, req.rating, req.review, req.tasting_date, now, now],
    )
    .map_err(|e| e.to_string())?;

    // 插入链接
    for link in &req.links {
        let link_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
            params![link_id, id, link.url, link.label],
        )
        .map_err(|e| e.to_string())?;
    }

    // 插入标签
    for tag in &req.tags {
        let tag_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
            params![tag_id, id, tag],
        )
        .map_err(|e| e.to_string())?;
    }

    drop(conn);
    get_entry(id)
}

#[tauri::command]
fn update_entry(req: UpdateEntryRequest) -> Result<Entry, String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;
    validate_entry_fields(&req.name, &req.genre_id, &req.rating, &req.review, &conn)?;
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "UPDATE entries SET name = ?1, genre_id = ?2, creator = ?3, rating = ?4, review = ?5,
         tasting_date = ?6, updated_at = ?7 WHERE id = ?8",
        params![req.name, req.genre_id, req.creator, req.rating, req.review, req.tasting_date, now, req.id],
    )
    .map_err(|e| e.to_string())?;

    // 更新链接：先删后插
    conn.execute("DELETE FROM external_links WHERE entry_id = ?", params![req.id])
        .map_err(|e| e.to_string())?;
    for link in &req.links {
        let link_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
            params![link_id, req.id, link.url, link.label],
        )
        .map_err(|e| e.to_string())?;
    }

    // 更新标签：先删后插
    conn.execute("DELETE FROM tags WHERE entry_id = ?", params![req.id])
        .map_err(|e| e.to_string())?;
    for tag in &req.tags {
        let tag_id = Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
            params![tag_id, req.id, tag],
        )
        .map_err(|e| e.to_string())?;
    }

    drop(conn);
    get_entry(req.id)
}

#[tauri::command]
fn delete_entries(ids: Vec<String>) -> Result<(), String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    for id in &ids {
        // 获取图片路径用于删除文件
        let mut stmt = conn
            .prepare("SELECT path FROM entry_images WHERE entry_id = ?")
            .map_err(|e| e.to_string())?;
        let paths: Vec<String> = stmt
            .query_map(params![id], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        drop(stmt);

        // 删除数据库记录（级联删除关联表）
        conn.execute("DELETE FROM entries WHERE id = ?", params![id])
            .map_err(|e| e.to_string())?;

        // 删除本地图片文件
        for path in paths {
            std::fs::remove_file(resolve_image_path(&path)).ok();
        }
    }

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
    let conn = DB.lock().map_err(|e| e.to_string())?;

    // 如果设为主图，先取消其他主图
    if is_primary {
        conn.execute(
            "UPDATE entry_images SET is_primary = 0 WHERE entry_id = ?",
            params![entry_id],
        )
        .map_err(|e| e.to_string())?;
    }

    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
        params![id, entry_id, path, is_primary as i32],
    )
    .map_err(|e| e.to_string())?;

    Ok(EntryImage {
        id,
        entry_id,
        path,
        is_primary,
    })
}

#[tauri::command]
fn delete_entry_image(id: String) -> Result<(), String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    // 获取图片路径
    let path: Option<String> = conn
        .query_row("SELECT path FROM entry_images WHERE id = ?", params![id], |row| row.get(0))
        .ok();

    conn.execute("DELETE FROM entry_images WHERE id = ?", params![id])
        .map_err(|e| e.to_string())?;

    // 删除文件
    if let Some(p) = path {
        std::fs::remove_file(resolve_image_path(&p)).ok();
    }

    Ok(())
}

#[tauri::command]
fn set_primary_image(id: String) -> Result<(), String> {
    let conn = DB.lock().map_err(|e| e.to_string())?;

    // 获取 entry_id
    let entry_id: String = conn
        .query_row("SELECT entry_id FROM entry_images WHERE id = ?", params![id], |row| row.get(0))
        .map_err(|e| e.to_string())?;

    // 取消该条目所有主图
    conn.execute(
        "UPDATE entry_images SET is_primary = 0 WHERE entry_id = ?",
        params![entry_id],
    )
    .map_err(|e| e.to_string())?;

    // 设置新主图
    conn.execute(
        "UPDATE entry_images SET is_primary = 1 WHERE id = ?",
        params![id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
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

    let entries: Vec<(String, String, String, Option<String>, String, String, Option<String>, String, String)> = stmt
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
            .query_map(params![id], |row| Ok(ExternalLink {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                url: row.get(2)?,
                label: row.get(3)?,
            }))
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
    let export_dir = project_root.join("exports");
    std::fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    let content = match format.as_str() {
        "json" => serde_json::to_string_pretty(&export_entries).map_err(|e| e.to_string())?,
        "csv" => {
            let mut csv = String::from("名称,类型,创作者,等级,评价,品鉴日期,标签,链接\n");
            for entry in &export_entries {
                let tags_str = entry.tags.join(";");
                let links_str = entry.links.iter().map(|l| format!("{}:{}", l.label, l.url)).collect::<Vec<_>>().join(";");
                csv.push_str(&format!(
                    "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"\n",
                    entry.name, entry.genre_name,
                    entry.creator.as_deref().unwrap_or(""),
                    entry.rating, entry.review.replace("\"", "\"\""),
                    entry.tasting_date.as_deref().unwrap_or(""),
                    tags_str, links_str
                ));
            }
            csv
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
                        let src = resolve_image_path(img_path);
                        if src.exists() {
                            cover_counter += 1;
                            let ext = std::path::Path::new(img_path)
                                .extension()
                                .and_then(|e| e.to_str())
                                .unwrap_or("jpg")
                                .to_lowercase();
                            let fname = format!("cover_{}.{}", cover_counter, ext);
                            if std::fs::copy(&src, export_dir.join(&fname)).is_ok() {
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
                html.push_str(&format!("<div class=\"entry\"><h2>{}</h2>\n", escape_html(&entry.name)));
                html.push_str(&format!(
                    "<p class=\"meta\">{} · <span class=\"rating\">{}</span> · {}{}</p>\n",
                    escape_html(&entry.genre_name),
                    escape_html(&entry.rating),
                    escape_html(entry.tasting_date.as_deref().unwrap_or("")),
                    entry.creator.as_deref().map(|c| format!(" · {}", escape_html(c))).unwrap_or_default()
                ));
                if !entry.tags.is_empty() {
                    html.push_str(&format!(
                        "<p class=\"meta\">标签：{}</p>\n",
                        entry.tags.iter().map(|t| escape_html(t)).collect::<Vec<_>>().join("、")
                    ));
                }
                // 图片：Base64 嵌入
                html.push_str("<div class=\"images\">");
                for img_path in &entry.images {
                    if let Ok(bytes) = std::fs::read(resolve_image_path(img_path)) {
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        let ext = std::path::Path::new(img_path)
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("jpg")
                            .to_lowercase();
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

    // 保存文件（markdown 用 .md 后缀）
    let ext = if format.as_str() == "markdown" { "md" } else { format.as_str() };
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let export_path = export_dir.join(format!("export_{}.{}", timestamp, ext));
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
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .timeout(std::time::Duration::from_secs(timeout_secs));

    let proxy_env = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.trim().is_empty()));
    if let Some(proxy) = proxy_env {
        if let Ok(p) = reqwest::Proxy::all(&proxy) {
            builder = builder.proxy(p);
        }
    }

    builder.build().map_err(|e| format!("创建 HTTP 客户端失败: {}", e))
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

    let html = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求失败: {}", e))?
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

    let resp = client
        .get(&url)
        .header("Referer", format!("https://{}.douban.com/", cat))
        .send()
        .map_err(|e| format!("请求失败: {}", e))?;

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

    let html = req
        .send()
        .map_err(|e| format!("请求失败: {}", e))?
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

    let resp = client
        .post("https://graphql.anilist.co")
        .json(&body)
        .send()
        .map_err(|e| format!("请求失败: {}", e))?;

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

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求失败: {}", e))?;
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
    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("请求失败: {}", e))?;
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

/// 下载封面图片到本地项目 resource/cover_image 目录
#[tauri::command]
fn download_cover(
    url: String,
    title: String,
    creator: Option<String>,
) -> Result<String, String> {
    let title_clean = sanitize_filename(&title);
    let creator_clean = match creator {
        Some(c) if !c.trim().is_empty() => format!("_{}", sanitize_filename(c.trim())),
        _ => String::new(),
    };

    let client = build_client(30)?;

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("下载失败: {}", e))?;
    let bytes = resp.bytes().map_err(|e| format!("读取失败: {}", e))?;

    // 从 URL 推断后缀
    let ext = guess_ext_from_url(&url).unwrap_or_else(|| "jpg".to_string());

    // 目标路径：项目根/resource/cover_image
    let project_root = get_project_root();
    let cover_dir = project_root.join("resource").join("cover_image");
    std::fs::create_dir_all(&cover_dir).map_err(|e| format!("创建目录失败: {}", e))?;

    // 处理文件名冲突：同名时附加 (1), (2)...
    let base = format!("{}{}.{}", title_clean, creator_clean, ext);
    let mut target = cover_dir.join(&base);
    let mut counter = 1u32;
    while target.exists() {
        let stem = format!("{}{} ({}).{}", title_clean, creator_clean, counter, ext);
        target = cover_dir.join(stem);
        counter += 1;
    }

    std::fs::write(&target, &bytes).map_err(|e| format!("写入失败: {}", e))?;

    Ok(to_project_rel_path(&target))
}

/// 处理拖入的本地图片：复制到 cover_image 目录并重命名
#[tauri::command]
fn import_local_image(source_path: String, title: String, creator: Option<String>) -> Result<String, String> {
    // 获取文件后缀
    let ext = std::path::Path::new(&source_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "jpg".to_string());

    // 清理文件名
    let title_clean = if title.is_empty() {
        "未命名".to_string()
    } else {
        sanitize_filename(&title)
    };
    let creator_clean = creator
        .as_ref()
        .filter(|c| !c.is_empty())
        .map(|c| sanitize_filename(c))
        .unwrap_or_else(|| "未知作者".to_string());

    // 目标目录
    let project_root = get_project_root();
    let cover_dir = project_root.join("resource").join("cover_image");
    std::fs::create_dir_all(&cover_dir).map_err(|e| format!("创建目录失败: {}", e))?;

    // 构建文件名
    let base = format!("{}_{}.{}", title_clean, creator_clean, ext);
    let mut target = cover_dir.join(&base);
    let mut counter = 1u32;
    while target.exists() {
        let stem = format!("{}_{} ({}).{}", title_clean, creator_clean, counter, ext);
        target = cover_dir.join(stem);
        counter += 1;
    }

    // 复制文件
    std::fs::copy(&source_path, &target).map_err(|e| format!("复制文件失败: {}", e))?;

    Ok(to_project_rel_path(&target))
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

fn guess_ext_from_url(url: &str) -> Option<String> {
    // 去除 query string
    let path = url.split('?').next().unwrap_or(url);
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?;
    let lower = ext.to_lowercase();
    if ["jpg", "jpeg", "png", "webp", "gif", "bmp"].contains(&lower.as_str()) {
        Some(if lower == "jpeg" { "jpg".to_string() } else { lower })
    } else {
        None
    }
}

#[tauri::command]
fn backup_database() -> Result<String, String> {
    let db_path = get_db_path();
    let project_root = get_project_root();
    let backup_dir = project_root.join("backups");
    std::fs::create_dir_all(&backup_dir).map_err(|e| e.to_string())?;

    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = backup_dir.join(format!("backup_{}.db", timestamp));

    std::fs::copy(&db_path, &backup_path).map_err(|e| e.to_string())?;

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
    let src = Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| format!("无法打开源数据库: {}", e))?;
    let backup = rusqlite::backup::Backup::new(&src, dst)
        .map_err(|e| format!("创建导入会话失败: {}", e))?;
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
    import_db_into(&source_path, &mut dst)
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
                "SELECT e.rating, COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{} GROUP BY e.rating ORDER BY e.rating",
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
        let mut stmt = conn
            .prepare(&format!(
                "SELECT strftime('%Y', e.tasting_date), COUNT(DISTINCT e.id) FROM entries e JOIN genres g ON e.genre_id = g.id{} WHERE e.tasting_date IS NOT NULL AND e.tasting_date != '' GROUP BY 1 ORDER BY 1",
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
    if let Ok(id) = conn.query_row(
        "SELECT id FROM genres WHERE name = ?1",
        params![name],
        |row| row.get(0),
    ) {
        return Ok(id);
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
        out.push(ExportEntry {
            name,
            genre_name: genre,
            creator,
            rating,
            review,
            tasting_date,
            links,
            tags,
            images: vec![],
        });
    }
    Ok(out)
}

/// 导入 JSON/CSV 文件（批量新增条目，类型缺失自动创建）
#[tauri::command]
fn import_entries(path: String, format: String) -> Result<ImportResult, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("读取文件失败: {}", e))?;
    let conn = DB.lock().map_err(|e| e.to_string())?;

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
        let result = (|| -> Result<(), String> {
            // 类型：查找或创建
            let genre_id = find_or_create_genre(&conn, &rec.genre_name)?;
            // 字段校验
            validate_entry_fields(&rec.name, &genre_id, &rec.rating, &rec.review, &conn)?;

            let id = Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            conn.execute(
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
                conn.execute(
                    "INSERT INTO external_links (id, entry_id, url, label) VALUES (?1, ?2, ?3, ?4)",
                    params![Uuid::new_v4().to_string(), id, link.url, link.label],
                )
                .map_err(|e| e.to_string())?;
            }
            for tag in &rec.tags {
                if tag.trim().is_empty() {
                    continue;
                }
                conn.execute(
                    "INSERT INTO tags (id, entry_id, name) VALUES (?1, ?2, ?3)",
                    params![Uuid::new_v4().to_string(), id, tag],
                )
                .map_err(|e| e.to_string())?;
            }
            for (i, img) in rec.images.iter().enumerate() {
                if img.trim().is_empty() {
                    continue;
                }
                conn.execute(
                    "INSERT INTO entry_images (id, entry_id, path, is_primary) VALUES (?1, ?2, ?3, ?4)",
                    params![Uuid::new_v4().to_string(), id, img, if i == 0 { 1 } else { 0 }],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })();

        match result {
            Ok(()) => imported += 1,
            Err(e) => {
                failed += 1;
                if errors.len() < 20 {
                    errors.push(format!("《{}》: {}", rec.name, e));
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

#[tauri::command]
fn get_image_base64(path: String) -> Result<String, String> {
    let bytes = std::fs::read(resolve_image_path(&path))
        .map_err(|e| format!("读取图片失败: {}", e))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&bytes))
}

/// 保存前端生成的图片（分享卡片等）：data URL → 文件
#[tauri::command]
fn save_base64_image(data_url: String, path: String) -> Result<(), String> {
    let b64 = data_url
        .split(',')
        .nth(1)
        .ok_or_else(|| "无效的图片数据".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("图片数据解码失败: {}", e))?;
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
            download_cover,
            import_local_image,
            get_image_base64,
            save_base64_image,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|_app_handle, event| match event {
        tauri::RunEvent::ExitRequested { .. } => {
            // 关闭时自动备份数据库
            if let Err(e) = backup_database() {
                eprintln!("[WARN] 退出时自动备份失败: {}", e);
            }
        }
        _ => {}
    });
}

// ============================================================================
// 封面数据源网络测试（cargo test -- --nocapture）
// ============================================================================

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
            upgrade_cover_url("https://s4.anilist.co/file/anilistcdn/media/anime/cover/medium/bx1.jpg"),
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
        assert!(rel.to_string_lossy().ends_with("resource/cover_image/a.jpg"));

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
        let v: i64 = dst
            .query_row("SELECT x FROM t", [], |r| r.get(0))
            .unwrap();
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
