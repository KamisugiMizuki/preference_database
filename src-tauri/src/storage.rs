use rusqlite::{params, Connection, OpenFlags};
use std::io::Read;
use std::path::{Path, PathBuf};

/// 开发树继续原地使用；安装版固定写入用户目录，与工作目录无关。
pub(crate) fn prepare_data_root(exe: &Path, user_root: &Path) -> Result<PathBuf, String> {
    prepare_data_root_with_cwd(exe, user_root, std::env::current_dir().ok().as_deref())
}

fn prepare_data_root_with_cwd(
    exe: &Path,
    user_root: &Path,
    cwd: Option<&Path>,
) -> Result<PathBuf, String> {
    if let Some(root) = exe.ancestors().find(|candidate| {
        candidate.join("package.json").is_file()
            && candidate.join("src-tauri/tauri.conf.json").is_file()
    }) {
        return Ok(root.to_path_buf());
    }
    if user_root.join("database/database.db").is_file() {
        return Ok(user_root.to_path_buf());
    }
    // 旧版本按 exe 的第四层祖先存库；也兼容放在 exe 旁边的便携数据。
    let old_root = exe.ancestors().nth(4);
    let candidates = [exe.parent(), old_root];
    let legacy = candidates
        .into_iter()
        .flatten()
        .find(|path| path.join("database/database.db").is_file())
        .or_else(|| {
            // 旧版只有找不到第四层祖先才用 cwd；同名 SQLite 文件还须匹配旧表结构。
            let cwd = cwd.filter(|_| old_root.is_none())?;
            let source = Connection::open_with_flags(
                cwd.join("database/database.db"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .ok()?;
            let tables: i64 = source.query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE type = 'table' AND name IN ('genres', 'entries', 'external_links', 'tags', 'entry_images')",
                [],
                |row| row.get(0),
            ).ok()?;
            let schema_matches = tables == 5 && [
                "SELECT id, name, is_default, created_at FROM genres LIMIT 0",
                "SELECT id, name, genre_id, creator, rating, review, tasting_date, created_at, updated_at FROM entries LIMIT 0",
                "SELECT id, entry_id, url, label FROM external_links LIMIT 0",
                "SELECT id, entry_id, name FROM tags LIMIT 0",
                "SELECT id, entry_id, path, is_primary FROM entry_images LIMIT 0",
            ].iter().all(|sql| source.prepare(sql).is_ok());
            schema_matches.then_some(cwd)
        });
    if let Some(legacy) = legacy {
        migrate_legacy(legacy, user_root)?;
    } else {
        std::fs::create_dir_all(user_root).map_err(|e| e.to_string())?;
    }
    Ok(user_root.to_path_buf())
}

fn migrate_legacy(legacy: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target.join("database")).map_err(|e| e.to_string())?;
    let staging = target.join(format!("migration_{}.db", uuid::Uuid::new_v4()));
    let mut copies = Vec::new();
    let result = (|| -> Result<(), String> {
        let source = Connection::open_with_flags(
            legacy.join("database/database.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        super::backup_connection_to_path(&source, &staging)?;
        let mut destination = Connection::open(&staging).map_err(|e| e.to_string())?;
        let images: Vec<(String, String)> = destination
            .prepare("SELECT id, path FROM entry_images")
            .map_err(|e| e.to_string())?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        let tx = destination.transaction().map_err(|e| e.to_string())?;
        for (id, raw) in images {
            let old = legacy.join(&raw);
            // 只读文件头辨认格式；迁移旧文件不套用新下载的大小限制。
            let mut header = Vec::new();
            let ext = std::fs::File::open(&old)
                .and_then(|file| file.take(12).read_to_end(&mut header))
                .ok()
                .and_then(|_| super::image_extension_from_bytes(&header));
            let path = if let Some(ext) = ext {
                let relative = format!(
                    "resource/cover_image/migrated_{}.{}",
                    uuid::Uuid::new_v4(),
                    ext
                );
                let new = target.join(&relative);
                std::fs::create_dir_all(new.parent().unwrap()).map_err(|e| e.to_string())?;
                copies.push(new.clone());
                std::fs::copy(&old, new).map_err(|e| e.to_string())?;
                relative
            } else {
                eprintln!(
                    "[WARN] 旧封面缺失、不可读取或格式不受支持，保留原位置: {}",
                    old.display()
                );
                old.to_string_lossy().into_owned()
            };
            tx.execute(
                "UPDATE entry_images SET path = ?1 WHERE id = ?2",
                params![path, id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        drop(destination);
        std::fs::rename(&staging, target.join("database/database.db"))
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(staging);
        for path in copies {
            let _ = std::fs::remove_file(path);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_root_migrates_legacy_database_and_images_once() {
        let dir = std::env::temp_dir().join(format!("prefdb_migrate_{}", uuid::Uuid::new_v4()));
        let old = dir.join("old");
        let new = dir.join("user/data");
        std::fs::create_dir_all(old.join("database")).unwrap();
        std::fs::create_dir_all(old.join("resource/cover_image")).unwrap();
        let db = Connection::open(old.join("database/database.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        super::super::init_database(&db).unwrap();
        db.execute_batch("INSERT INTO entries (id,name,genre_id,rating,review,created_at,updated_at) SELECT 'e','legacy',id,'S','a long enough review','now','now' FROM genres LIMIT 1; INSERT INTO entry_images VALUES('i','e','resource/cover_image/a.png',1);").unwrap();
        std::fs::write(old.join("resource/cover_image/a.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        let writer = Connection::open(old.join("database/database.db")).unwrap();
        writer.execute_batch("BEGIN; INSERT INTO entries (id,name,genre_id,rating,review,created_at,updated_at) SELECT 'pending','uncommitted',id,'S','a long enough review','now','now' FROM genres LIMIT 1;").unwrap();
        assert!(
            old.join("database/database.db-wal")
                .metadata()
                .unwrap()
                .len()
                > 0
        );
        let exe = old.join("a/b/c/app.exe");
        assert_eq!(prepare_data_root(&exe, &new).unwrap(), new);
        let migrated = Connection::open(new.join("database/database.db")).unwrap();
        let image: String = migrated
            .query_row("SELECT path FROM entry_images", [], |r| r.get(0))
            .unwrap();
        assert!(new.join(image).is_file());
        assert!(old.join("resource/cover_image/a.png").is_file());
        assert_eq!(
            migrated
                .query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
        migrated
            .execute("UPDATE entries SET name = 'user copy'", [])
            .unwrap();
        assert_eq!(prepare_data_root(&exe, &new).unwrap(), new);
        assert_eq!(
            migrated
                .query_row("SELECT name FROM entries", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "user copy"
        );
        assert_eq!(
            migrated
                .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        writer.execute_batch("ROLLBACK").unwrap();
        drop((migrated, writer, db));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn legacy_image_limits_do_not_block_metadata_migration() {
        let dir = std::env::temp_dir().join(format!("prefdb_old_images_{}", uuid::Uuid::new_v4()));
        let old = dir.join("old");
        let new = dir.join("user");
        std::fs::create_dir_all(old.join("database")).unwrap();
        std::fs::create_dir_all(old.join("resource/cover_image")).unwrap();
        let db = Connection::open(old.join("database/database.db")).unwrap();
        super::super::init_database(&db).unwrap();
        db.execute_batch("INSERT INTO entries (id,name,genre_id,rating,review,created_at,updated_at) SELECT 'e','legacy',id,'S','a long enough review','now','now' FROM genres LIMIT 1;").unwrap();
        let mut large = vec![0; 10 * 1024 * 1024 + 1];
        large[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let images = [
            ("large.png", Some(large), true),
            ("old.tiff", Some(b"II*\0 legacy cover".to_vec()), false),
            ("missing.jpg", None, false),
        ];
        for (name, bytes, _) in &images {
            let raw = format!("resource/cover_image/{name}");
            if let Some(bytes) = bytes {
                std::fs::write(old.join(&raw), bytes).unwrap();
            }
            db.execute(
                "INSERT INTO entry_images VALUES (?1, 'e', ?2, 1)",
                params![name, raw],
            )
            .unwrap();
        }
        let result = prepare_data_root(&old.join("a/b/c/app.exe"), &new);
        if result.is_ok() {
            let migrated = Connection::open(new.join("database/database.db")).unwrap();
            assert_eq!(
                migrated
                    .query_row("SELECT name FROM entries", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "legacy"
            );
            assert_eq!(
                migrated
                    .query_row("SELECT COUNT(*) FROM entry_images", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                3
            );
            for (name, bytes, copied) in &images {
                let raw = format!("resource/cover_image/{name}");
                let path: String = migrated
                    .query_row("SELECT path FROM entry_images WHERE id = ?1", [name], |r| {
                        r.get(0)
                    })
                    .unwrap();
                if let Some(bytes) = bytes {
                    assert_eq!(&std::fs::read(old.join(&raw)).unwrap(), bytes);
                }
                if *copied {
                    assert!(Path::new(&path).is_relative());
                    assert_eq!(
                        std::fs::read(new.join(&path)).unwrap(),
                        *bytes.as_ref().unwrap()
                    );
                } else {
                    assert_eq!(Path::new(&path), old.join(&raw));
                    assert!(Path::new(&path).is_absolute());
                }
                let source_path: String = db
                    .query_row("SELECT path FROM entry_images WHERE id = ?1", [name], |r| {
                        r.get(0)
                    })
                    .unwrap();
                assert_eq!(source_path, raw);
            }
        }
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            result.is_ok(),
            "legacy metadata must stay readable: {result:?}"
        );
    }

    #[test]
    fn shallow_install_migrates_cwd_legacy_database() {
        let dir = std::env::temp_dir().join(format!("prefdb_cwd_{}", uuid::Uuid::new_v4()));
        let old = dir.join("old_working_directory");
        let new = dir.join("user");
        std::fs::create_dir_all(old.join("database")).unwrap();
        let db = Connection::open(old.join("database/database.db")).unwrap();
        super::super::init_database(&db).unwrap();
        db.execute_batch("INSERT INTO entries (id,name,genre_id,rating,review,created_at,updated_at) SELECT 'e','from cwd',id,'S','a long enough review','now','now' FROM genres LIMIT 1;").unwrap();
        // 合成浅层路径，不在盘符根目录创建文件或更改进程 cwd。
        let exe = dir
            .ancestors()
            .last()
            .unwrap()
            .join(format!("missing_install_{}/app.exe", uuid::Uuid::new_v4()));
        assert!(exe.ancestors().nth(4).is_none());
        assert!(!exe.parent().unwrap().exists());
        assert_eq!(
            prepare_data_root_with_cwd(&exe, &new, Some(&old)).unwrap(),
            new
        );
        let migrated = new.join("database/database.db");
        let exists = migrated.is_file();
        if exists {
            let migrated = Connection::open(migrated).unwrap();
            assert_eq!(
                migrated
                    .query_row("SELECT name FROM entries", [], |r| r.get::<_, String>(0))
                    .unwrap(),
                "from cwd"
            );
        }
        assert!(old.join("database/database.db").is_file());
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
        assert!(
            exists,
            "the old cwd database must be migrated for a shallow install"
        );
    }

    #[test]
    fn cwd_probe_requires_shallow_install_and_legacy_schema() {
        let dir = std::env::temp_dir().join(format!("prefdb_cwd_guard_{}", uuid::Uuid::new_v4()));
        let old = dir.join("cwd");
        std::fs::create_dir_all(old.join("database")).unwrap();
        let db = Connection::open(old.join("database/database.db")).unwrap();
        super::super::init_database(&db).unwrap();
        let deep_exe = dir.join("installation/a/b/c/app.exe");
        let deep_user = dir.join("deep_user");
        assert!(deep_exe.ancestors().nth(4).is_some());
        assert_eq!(
            prepare_data_root_with_cwd(&deep_exe, &deep_user, Some(&old)).unwrap(),
            deep_user
        );
        assert!(!deep_user.join("database/database.db").exists());

        let shallow_exe = dir
            .ancestors()
            .last()
            .unwrap()
            .join(format!("missing_install_{}/app.exe", uuid::Uuid::new_v4()));
        for (case, sql) in [
            ("wrong_columns", "DROP TABLE entry_images; CREATE TABLE entry_images (unrelated TEXT)"),
            ("view", "DROP TABLE entry_images; CREATE VIEW entry_images AS SELECT '' AS id, '' AS entry_id, '' AS path, 0 AS is_primary"),
        ] {
            db.execute_batch(sql).unwrap();
            let user = dir.join(case);
            assert_eq!(
                prepare_data_root_with_cwd(&shallow_exe, &user, Some(&old)).unwrap(),
                user
            );
            assert!(!user.join("database/database.db").exists());
        }
        drop(db);
        std::fs::write(old.join("database/database.db"), b"not sqlite").unwrap();
        let user = dir.join("corrupt");
        assert_eq!(
            prepare_data_root_with_cwd(&shallow_exe, &user, Some(&old)).unwrap(),
            user
        );
        assert!(!user.join("database/database.db").exists());
        assert_eq!(
            std::fs::read(old.join("database/database.db")).unwrap(),
            b"not sqlite"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failed_publish_removes_staging_and_copies_without_touching_legacy() {
        let dir = std::env::temp_dir().join(format!("prefdb_rollback_{}", uuid::Uuid::new_v4()));
        let old = dir.join("old");
        let new = dir.join("user");
        std::fs::create_dir_all(old.join("database")).unwrap();
        std::fs::create_dir_all(old.join("resource/cover_image")).unwrap();
        let db = Connection::open(old.join("database/database.db")).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
            .unwrap();
        super::super::init_database(&db).unwrap();
        db.execute_batch("INSERT INTO entries (id,name,genre_id,rating,review,created_at,updated_at) SELECT 'e','legacy',id,'S','a long enough review','now','now' FROM genres LIMIT 1; INSERT INTO entry_images VALUES('i','e','resource/cover_image/a.png',1);").unwrap();
        std::fs::write(old.join("resource/cover_image/a.png"), b"\x89PNG\r\n\x1a\n").unwrap();
        // 用目录阻挡最后的 rename，确保图片复制后也会完整回滚。
        std::fs::create_dir_all(new.join("database/database.db")).unwrap();
        let result = prepare_data_root(&old.join("a/b/c/app.exe"), &new);
        assert!(result.is_err());
        assert!(new.join("database/database.db").is_dir());
        assert!(std::fs::read_dir(new.join("resource/cover_image"))
            .unwrap()
            .next()
            .is_none());
        assert!(!std::fs::read_dir(&new).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("migration_")));
        assert_eq!(
            std::fs::read(old.join("resource/cover_image/a.png")).unwrap(),
            b"\x89PNG\r\n\x1a\n"
        );
        assert_eq!(
            db.query_row("SELECT path FROM entry_images", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "resource/cover_image/a.png"
        );
        assert_eq!(
            db.query_row("SELECT name FROM entries", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "legacy"
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn development_root_and_failed_migration_preserve_data() {
        let dir = std::env::temp_dir().join(format!("prefdb_paths_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src-tauri")).unwrap();
        std::fs::write(dir.join("package.json"), "{}").unwrap();
        std::fs::write(dir.join("src-tauri/tauri.conf.json"), "{}").unwrap();
        let user = dir.join("user");
        assert_eq!(
            prepare_data_root(&dir.join("src-tauri/target/debug/app.exe"), &user).unwrap(),
            dir
        );
        std::fs::remove_file(dir.join("package.json")).unwrap();
        std::fs::create_dir_all(dir.join("database")).unwrap();
        std::fs::write(dir.join("database/database.db"), "not a database").unwrap();
        assert!(prepare_data_root(&dir.join("a/b/c/app.exe"), &user).is_err());
        assert!(!user.join("database/database.db").exists());
        assert!(dir.join("database/database.db").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
