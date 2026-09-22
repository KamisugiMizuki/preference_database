use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

static COMMAND_TEST_LOCK: Mutex<()> = Mutex::new(());
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

fn serve(
    responses: Vec<(u16, &'static str, Vec<u8>)>,
) -> (String, std::thread::JoinHandle<Vec<Instant>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/cover.jpg", listener.local_addr().unwrap());
    let task = std::thread::spawn(move || {
        let mut arrivals = Vec::new();
        for (status, mime, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "HTTP fixture timed out");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 16384);
            }
            arrivals.push(Instant::now());
            write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            let _ = socket.write_all(&body);
        }
        arrivals
    });
    (url, task)
}

#[test]
fn preview_uses_validated_data_url_and_host_throttle() {
    let (url, server) = serve(vec![
        (200, "image/png", PNG.to_vec()),
        (200, "image/png", PNG.to_vec()),
    ]);
    for _ in 0..2 {
        assert_eq!(
            fetch_cover_preview_data(&url).unwrap(),
            format!(
                "data:image/png;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(PNG)
            )
        );
    }
    let arrivals = server.join().unwrap();
    assert!(arrivals[1].duration_since(arrivals[0]) >= Duration::from_secs(1));
}

#[test]
fn download_rejects_html_and_uses_content_extension() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let (url, server) = serve(vec![
        (200, "text/html", b"<html>not an image</html>".to_vec()),
        (200, "image/png", PNG.to_vec()),
    ]);
    let rejected = download_cover(url.clone(), "bad-response".into(), None);
    if let Ok(path) = &rejected {
        std::fs::remove_file(resolve_image_path(path)).unwrap();
    }
    let accepted = download_cover(url, "actual-png".into(), None).unwrap();
    let bytes = std::fs::read(resolve_image_path(&accepted)).unwrap();
    std::fs::remove_file(resolve_image_path(&accepted)).unwrap();
    server.join().unwrap();
    assert!(
        rejected.is_err(),
        "HTML must never be persisted as an image"
    );
    assert!(accepted.ends_with(".png"), "extension must follow content");
    assert_eq!(bytes, PNG);
}

#[test]
fn http_errors_mime_mismatch_size_and_rate_are_enforced() {
    let (url, server) = serve(vec![
        (404, "image/png", PNG.to_vec()),
        (200, "image/jpeg", PNG.to_vec()),
        (200, "image/png", vec![0; 10 * 1024 * 1024 + 1]),
        (200, "image/png", PNG.to_vec()),
    ]);
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .build()
        .unwrap();
    assert!(send_rate_limited(client.get(&url), &url).is_err());
    assert!(validate_image_response(send_rate_limited(client.get(&url), &url).unwrap()).is_err());
    assert!(validate_image_response(send_rate_limited(client.get(&url), &url).unwrap()).is_err());
    assert_eq!(
        validate_image_response(send_rate_limited(client.get(&url), &url).unwrap())
            .unwrap()
            .1,
        "png"
    );
    let arrivals = server.join().unwrap();
    for pair in arrivals.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= Duration::from_secs(1));
    }
}

fn reset_commands() {
    let conn = DB.lock().unwrap();
    conn.execute("DELETE FROM entries", []).unwrap();
    conn.execute("DELETE FROM genres WHERE is_default = 0", [])
        .unwrap();
    let dir = get_project_root().join("resource/cover_image");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(dir).unwrap();
}

fn request(name: &str, paths: Vec<String>) -> CreateEntryRequest {
    CreateEntryRequest {
        name: name.into(),
        genre_id: get_genres().unwrap()[0].id.clone(),
        creator: None,
        rating: "S".into(),
        review: "用于行为测试的完整评价文本".into(),
        tasting_date: None,
        links: vec![],
        tags: vec!["测试".into()],
        image_paths: paths,
    }
}

fn stored_image(name: &str) -> String {
    let path = format!("resource/cover_image/{name}.png");
    std::fs::write(resolve_image_path(&path), PNG).unwrap();
    path
}

#[test]
fn import_failure_cleans_partial_copies_and_rolls_back_genre() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let dir = get_project_root().join("import_fixture");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ok.png"), PNG).unwrap();
    let records = serde_json::json!([
        {"name":"bad image", "genre_name":"new failed genre", "rating":"S", "review":"这是一段足够长的评价文段", "creator":null,"tasting_date":null,"links":[],"tags":[],"images":["ok.png","missing.png"]},
        {"name":"bad review", "genre_name":"new failed genre", "rating":"S", "review":"短", "creator":null,"tasting_date":null,"links":[],"tags":[],"images":["ok.png"]}
    ]);
    let input = dir.join("import.json");
    std::fs::write(&input, records.to_string()).unwrap();
    let result = import_entries(input.to_string_lossy().into(), "json".into()).unwrap();
    assert_eq!((result.imported, result.failed), (0, 2));
    assert_eq!(get_entries_count(None).unwrap(), 0);
    assert!(!get_genres()
        .unwrap()
        .iter()
        .any(|g| g.name == "new failed genre"));
    assert_eq!(
        std::fs::read_dir(get_project_root().join("resource/cover_image"))
            .unwrap()
            .count(),
        0,
        "failed image batch must not leave copies"
    );
}

#[test]
fn legacy_absolute_managed_image_is_removed_after_last_reference() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("legacy_absolute");
    let entry = create_entry(request("legacy", vec![path.clone()])).unwrap();
    DB.lock()
        .unwrap()
        .execute(
            "UPDATE entry_images SET path=?1",
            params![resolve_image_path(&path).to_string_lossy()],
        )
        .unwrap();
    delete_entries(vec![entry.id]).unwrap();
    assert!(!resolve_image_path(&path).exists());
}

#[cfg(windows)]
#[test]
fn locked_image_is_retried_after_handle_is_released() {
    use std::os::windows::fs::OpenOptionsExt;
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("locked");
    let entry = create_entry(request("locked", vec![path.clone()])).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(resolve_image_path(&path))
        .unwrap();
    delete_entries(vec![entry.id.clone()]).unwrap();
    assert!(resolve_image_path(&path).exists());
    drop(lock);
    delete_entries(vec![entry.id]).unwrap();
    assert!(
        !resolve_image_path(&path).exists(),
        "committed cleanup must survive the lost entry row"
    );
}

#[test]
fn shared_image_alias_survives_until_last_reference() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("shared");
    let a = create_entry(request("a", vec![path.clone()])).unwrap();
    let alias = path.replace('/', "\\");
    let b = create_entry(request("b", vec![alias])).unwrap();
    delete_entries(vec![a.id]).unwrap();
    assert!(
        resolve_image_path(&path).exists(),
        "slash aliases must share ownership"
    );
    delete_entry_image(b.images[0].id.clone()).unwrap();
    assert!(!resolve_image_path(&path).exists());
}

#[test]
fn failed_update_preserves_old_data_and_new_primary_is_selected() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let old = stored_image("old");
    let entry = create_entry(request("before", vec![old.clone()])).unwrap();
    let update = UpdateEntryRequest {
        id: entry.id.clone(),
        name: "after".into(),
        genre_id: entry.genre_id.clone(),
        creator: None,
        rating: "A".into(),
        review: entry.review.clone(),
        tasting_date: None,
        links: vec![],
        tags: vec!["new".into()],
        new_image_paths: vec!["../escape.png".into()],
        removed_image_ids: vec![entry.images[0].id.clone()],
    };
    assert!(update_entry(update.clone()).is_err());
    let unchanged = get_entry(entry.id.clone()).unwrap();
    assert_eq!(unchanged.name, "before");
    assert_eq!(unchanged.tags, vec!["测试"]);
    assert!(resolve_image_path(&old).exists());
    let new = stored_image("replacement");
    let changed = update_entry(UpdateEntryRequest {
        new_image_paths: vec![new],
        ..update
    })
    .unwrap();
    assert_eq!(changed.images.len(), 1);
    assert!(
        changed.images[0].is_primary,
        "removing primary and adding image must promote remaining image"
    );
    assert!(!resolve_image_path(&old).exists());
}

#[test]
fn image_commands_reject_outside_paths_and_non_images() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let outside = get_project_root().join("outside.png");
    std::fs::write(&outside, PNG).unwrap();
    assert!(get_image_base64(outside.to_string_lossy().into()).is_err());
    assert!(get_image_base64("resource/cover_image/../../outside.png".into()).is_err());
    let invalid = get_project_root().join("fake.jpg");
    std::fs::write(&invalid, "<html>not an image</html>").unwrap();
    assert!(import_local_image(invalid.to_string_lossy().into(), "bad".into(), None).is_err());
    let data = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(PNG)
    );
    let dangerous = get_project_root().join("database/database.db");
    assert!(save_base64_image(data.clone(), dangerous.to_string_lossy().into()).is_err());
    let output = get_project_root().join("share.png");
    save_base64_image(data, output.to_string_lossy().into()).unwrap();
    assert_eq!(std::fs::read(output).unwrap(), PNG);
}

#[test]
fn csv_export_import_roundtrip_preserves_independent_images() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let source = stored_image("csv,带引号");
    let original = create_entry(request("带图片的CSV", vec![source.clone()])).unwrap();
    let exported = export_entries("all".into(), "csv".into(), true, None, None).unwrap();
    let records = parse_import_csv(&std::fs::read_to_string(&exported).unwrap()).unwrap();
    assert_eq!(
        records[0].images.len(),
        1,
        "CSV must consume the exported image column"
    );
    delete_entries(vec![original.id]).unwrap();
    assert!(!resolve_image_path(&source).exists());
    let result = import_entries(exported, "csv".into()).unwrap();
    assert_eq!((result.imported, result.failed), (1, 0));
    let copy: String = DB
        .lock()
        .unwrap()
        .query_row("SELECT path FROM entry_images", [], |r| r.get(0))
        .unwrap();
    assert!(resolve_image_path(&copy).is_file());
}

#[test]
fn imported_images_have_independent_ownership() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("source");
    let input = get_project_root().join("ownership.json");
    let record = serde_json::json!({"name":"imported", "genre_name":"custom", "rating":"S", "review":"用于行为测试的一段长评价文本", "creator":null,"tasting_date":null,"links":[],"tags":[],"images":[path]});
    std::fs::write(
        &input,
        serde_json::json!([record.clone(), record]).to_string(),
    )
    .unwrap();
    let result = import_entries(input.to_string_lossy().into(), "json".into()).unwrap();
    assert_eq!((result.imported, result.failed), (2, 0));
    let rows: Vec<(String, String)> = DB
        .lock()
        .unwrap()
        .prepare("SELECT entry_id,path FROM entry_images")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_ne!(rows[0].1, rows[1].1);
    delete_entries(vec![rows[0].0.clone()]).unwrap();
    assert!(resolve_image_path(&rows[1].1).is_file());
    assert!(resolve_image_path(&path).is_file());
}

#[test]
fn delete_batch_failure_rolls_back_database_and_preserves_files() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("batch");
    let a = create_entry(request("a", vec![path.clone()])).unwrap();
    let b = create_entry(request("blocked", vec![])).unwrap();
    DB.lock().unwrap().execute_batch("CREATE TRIGGER test_reject_delete BEFORE DELETE ON entries WHEN OLD.name = 'blocked' BEGIN SELECT RAISE(ABORT, 'test failure'); END;").unwrap();
    let result = delete_entries(vec![a.id.clone(), b.id]);
    DB.lock()
        .unwrap()
        .execute_batch("DROP TRIGGER test_reject_delete")
        .unwrap();
    assert!(result.is_err());
    assert_eq!(get_entries_count(None).unwrap(), 2);
    assert_eq!(get_entry(a.id).unwrap().images.len(), 1);
    assert!(resolve_image_path(&path).is_file());
}

#[test]
fn invalid_url_rolls_back_new_images_and_preserves_existing_entry() {
    let _guard = COMMAND_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    reset_commands();
    let path = stored_image("url_rejected");
    let mut req = request("invalid", vec![path.clone()]);
    req.links.push(ExternalLink {
        id: String::new(),
        entry_id: String::new(),
        label: "bad".into(),
        url: "javascript:alert(1)".into(),
    });
    assert!(create_entry(req).is_err());
    assert_eq!(get_entries_count(None).unwrap(), 0);
    assert!(!resolve_image_path(&path).is_file());
    for url in [
        "file:///C:/test.png",
        "data:text/html,test",
        "java\nscript:alert(1)",
    ] {
        assert!(validate_http_url(url).is_err());
    }
    assert!(validate_http_url("https://example.com/中文?q=a&b=1").is_ok());
}

#[test]
fn backup_includes_committed_wal_and_excludes_uncommitted_data() {
    let dir = get_project_root().join(format!("wal_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("source.db");
    let source = Connection::open(&db).unwrap();
    source.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE t(x); INSERT INTO t VALUES(42);").unwrap();
    let writer = Connection::open(&db).unwrap();
    writer
        .execute_batch("BEGIN; INSERT INTO t VALUES(99);")
        .unwrap();
    let backup = dir.join("backup.db");
    backup_connection_to_path(&source, &backup).unwrap();
    let copy = Connection::open(&backup).unwrap();
    let values: Vec<i64> = copy
        .prepare("SELECT x FROM t")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(values, vec![42]);
    let check: String = copy
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(check, "ok");
    writer.execute_batch("ROLLBACK").unwrap();
    drop((copy, writer, source));
    std::fs::remove_dir_all(dir).unwrap();
}
