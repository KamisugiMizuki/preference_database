# database/

SQLite 数据库目录，程序启动时自动创建。

| 文件 | 说明 |
| --- | --- |
| `database.db` | 主数据库。包含全部品鉴记录（作品名称、个人评价全文、标签、图片路径等），**不入库** |
| `database.db-wal` / `database.db-shm` | SQLite WAL 模式临时文件，运行时出现，关闭后自动清理 |

表结构由 `src-tauri/src/lib.rs` 的 `init_database()` 维护：
`genres`、`entries`、`external_links`、`tags`、`entry_images`。

如需空库结构示例，直接运行程序会自动生成；不要手动提交 `.db` 文件。
