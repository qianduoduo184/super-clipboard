use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clipboard::types::ClipboardItemDraft;

use super::schema::{INDEX_SQL, INIT_SQL};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: String,
    pub hash: String,
    pub item_type: String,
    pub content: Option<String>,
    pub content_path: Option<String>,
    pub preview: String,
    pub source_app: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[cfg(test)]
mod tests {
    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};

    use super::*;

    fn open_test_repository() -> ClipboardRepository {
        let path = std::env::temp_dir().join(format!("super-clipboard-{}.sqlite3", Uuid::new_v4()));
        ClipboardRepository::open(path).expect("repository")
    }

    fn text_draft(content: &str) -> ClipboardItemDraft {
        ClipboardItemDraft {
            item_type: ClipboardItemType::Text,
            content: Some(content.to_string()),
            content_path: None,
            preview: content.to_string(),
            source_app: Some("test".to_string()),
            size_bytes: content.len() as i64,
        }
    }

    #[test]
    fn insert_or_touch_deduplicates_by_hash() {
        let repository = open_test_repository();

        let first = repository
            .insert_or_touch(text_draft("hello"))
            .expect("first insert");
        let second = repository
            .insert_or_touch(text_draft("hello"))
            .expect("second insert");

        assert_eq!(first.id, second.id);
    }

    #[test]
    fn search_returns_matching_items() {
        let repository = open_test_repository();
        repository
            .insert_or_touch(text_draft("sqlite clipboard history"))
            .expect("insert");

        let results = repository
            .search(
                "sqlite".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].preview, "sqlite clipboard history");
    }

    #[test]
    fn search_handles_fts_special_characters_without_error() {
        let repository = open_test_repository();
        repository
            .insert_or_touch(text_draft(
                "open http://example.com/path and keep \"quoted\" text",
            ))
            .expect("insert");

        // With trigram tokenizer, search for shorter substrings (3+ chars) from the URL
        let url_results = repository
            .search(
                "example".to_string(),  // Search for domain name keyword instead of full URL
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("url search");
        let punctuation_results = repository
            .search(
                "\"unterminated (AND OR NOT *)".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("punctuation search");

        // Trigram tokenizer enables substring matching for 3+ character queries
        assert_eq!(url_results.len(), 1);
        // Punctuation is stripped by to_fts_query, so this won't match but shouldn't error
        assert!(punctuation_results.is_empty());
    }

    #[test]
    fn debug_fts5_cjk_tokenization() {
        use rusqlite::Connection;

        let conn = Connection::open_in_memory().expect("connection");
        conn.execute(
            "CREATE VIRTUAL TABLE test_fts USING fts5(content, tokenize='trigram')",
            [],
        ).expect("create table");

        conn.execute(
            "INSERT INTO test_fts(content) VALUES (?)",
            ["同步组织排序码到云之家"],
        ).expect("insert");

        // First, check if data is actually in the table
        let row_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM test_fts",
            [],
            |row| row.get(0),
        ).expect("count");
        println!("Total rows in FTS table: {}", row_count);

        // Check the actual content
        let content: String = conn.query_row(
            "SELECT content FROM test_fts",
            [],
            |row| row.get(0),
        ).expect("get content");
        println!("Stored content: {}", content);

        // Test trigram tokenizer with CJK text
        // Trigram creates 3-character tokens, ideal for CJK substring search

        // Test 1: Exact substring match (trigram should handle this)
        let q1 = r#""云之家""#;
        let c1: i64 = conn.query_row(
            "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
            [q1],
            |row| row.get(0),
        ).unwrap_or(0);
        println!("Query '{}' -> {}", q1, c1);

        // Test 2: Single character
        let q2 = r#""云""#;
        let c2: i64 = conn.query_row(
            "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
            [q2],
            |row| row.get(0),
        ).unwrap_or(0);
        println!("Query '{}' -> {}", q2, c2);

        // Test 3: Two characters
        let q3 = r#""云之""#;
        let c3: i64 = conn.query_row(
            "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
            [q3],
            |row| row.get(0),
        ).unwrap_or(0);
        println!("Query '{}' -> {}", q3, c3);

        assert!(c1 > 0, "Trigram tokenizer should match CJK substring");
    }

    #[test]
    fn search_finds_cjk_substring() {
        let repository = open_test_repository();
        repository
            .insert_or_touch(text_draft("同步组织排序码到云之家"))
            .expect("insert");

        let results = repository
            .search(
                "云之家".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].preview, "同步组织排序码到云之家");
    }

    #[test]
    fn search_finds_short_substring() {
        let repository = open_test_repository();

        // Insert test items with various patterns
        repository
            .insert_or_touch(text_draft("123ab332sddsdf"))
            .expect("insert 1");
        repository
            .insert_or_touch(text_draft("test ab content"))
            .expect("insert 2");
        repository
            .insert_or_touch(text_draft("no xyz here"))
            .expect("insert 3");

        // Test 1-char query (should use LIKE)
        let results_1char = repository
            .search(
                "x".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("1-char search");
        assert_eq!(results_1char.len(), 1, "Should find 'x' in one item");

        // Test 2-char query (should use LIKE) - the key test case
        let results_2char = repository
            .search(
                "ab".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("2-char search");
        assert_eq!(results_2char.len(), 2, "Should find 'ab' substring in two items (123ab332sddsdf and test ab content)");

        // Verify the specific items found
        let found_previews: Vec<&str> = results_2char.iter().map(|item| item.preview.as_str()).collect();
        assert!(found_previews.contains(&"123ab332sddsdf"), "Should find '123ab332sddsdf'");
        assert!(found_previews.contains(&"test ab content"), "Should find 'test ab content'");

        // Test 3-char query (should use FTS5 trigram)
        let results_3char = repository
            .search(
                "3ab".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("3-char search");
        assert_eq!(results_3char.len(), 1, "Should find '3ab' in one item via trigram");
        assert_eq!(results_3char[0].preview, "123ab332sddsdf");

        // Test substring in middle
        let results_middle = repository
            .search(
                "ab3".to_string(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("middle substring search");
        assert_eq!(results_middle.len(), 1, "Should find 'ab3' substring via trigram");
        assert_eq!(results_middle[0].preview, "123ab332sddsdf");
    }

    #[test]
    fn insert_or_touch_allows_reinserting_soft_deleted_content() {
        let repository = open_test_repository();

        let first = repository
            .insert_or_touch(text_draft("repeatable"))
            .expect("first insert");
        repository.soft_delete(&first.id).expect("soft delete");
        let second = repository
            .insert_or_touch(text_draft("repeatable"))
            .expect("second insert");

        assert_ne!(first.id, second.id);
    }

    #[test]
    fn prune_history_soft_deletes_old_non_favorites_over_limit() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch(text_draft("first"))
            .expect("first insert");
        let second = repository
            .insert_or_touch(text_draft("second"))
            .expect("second insert");
        let favorite = repository
            .insert_or_touch(text_draft("favorite"))
            .expect("favorite insert");
        repository.toggle_favorite(&favorite.id).expect("favorite");

        repository.prune_history(1, 0).expect("prune");

        let first_active = repository.get_item(&first.id).expect("first").is_some();
        let second_active = repository.get_item(&second.id).expect("second").is_some();
        assert_ne!(first_active, second_active);
        assert!(repository
            .get_item(&favorite.id)
            .expect("favorite")
            .is_some());
    }

    #[test]
    fn reorder_items_persists_custom_history_order() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch(text_draft("first"))
            .expect("first insert");
        let second = repository
            .insert_or_touch(text_draft("second"))
            .expect("second insert");
        let third = repository
            .insert_or_touch(text_draft("third"))
            .expect("third insert");

        repository
            .reorder_items(&[third.id.clone(), first.id.clone(), second.id.clone()])
            .expect("reorder");

        let results = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("search");

        assert_eq!(
            results
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec![third.id.as_str(), first.id.as_str(), second.id.as_str()]
        );
    }

    #[test]
    fn insert_or_touch_places_new_items_before_custom_order() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch(text_draft("first"))
            .expect("first insert");
        let second = repository
            .insert_or_touch(text_draft("second"))
            .expect("second insert");
        repository
            .reorder_items(&[first.id.clone(), second.id.clone()])
            .expect("reorder");

        let third = repository
            .insert_or_touch(text_draft("third"))
            .expect("third insert");
        let results = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("search");

        assert_eq!(results[0].id, third.id);
    }

    #[test]
    fn open_migrates_legacy_database_without_pinned_column() {
        let path =
            std::env::temp_dir().join(format!("super-clipboard-legacy-{}.sqlite3", Uuid::new_v4()));
        {
            let conn = Connection::open(&path).expect("legacy db");
            conn.execute_batch(
                r#"
                CREATE TABLE clipboard_items (
                  id TEXT PRIMARY KEY,
                  hash TEXT NOT NULL,
                  item_type TEXT NOT NULL,
                  content TEXT,
                  content_path TEXT,
                  preview TEXT NOT NULL,
                  source_app TEXT,
                  favorite INTEGER NOT NULL DEFAULT 0,
                  size_bytes INTEGER NOT NULL DEFAULT 0,
                  created_at INTEGER NOT NULL,
                  updated_at INTEGER NOT NULL,
                  deleted_at INTEGER
                );
                CREATE VIRTUAL TABLE clipboard_items_fts
                USING fts5(id UNINDEXED, preview, content);
                INSERT INTO clipboard_items
                (id, hash, item_type, content, preview, favorite, size_bytes, created_at, updated_at)
                VALUES ('legacy-id', 'legacy-hash', 'text', 'legacy text', 'legacy text', 0, 11, 1, 1);
                "#,
            )
            .expect("legacy schema");
        }

        let repository = ClipboardRepository::open(path).expect("open migrated db");
        let item = repository
            .get_item("legacy-id")
            .expect("get legacy item")
            .expect("legacy item exists");

        assert!(!item.pinned);
    }

    #[test]
    #[ignore] // Long-running test, run with: cargo test -- --ignored
    fn performance_test_1000_items_insert_and_search() {
        let repository = open_test_repository();

        // Insert 1,000 text items
        let start = std::time::Instant::now();
        for i in 0..1000 {
            repository
                .insert_or_touch(text_draft(&format!("测试条目 {}: 这是一段中文内容用于测试搜索性能", i)))
                .expect("insert");
        }
        let insert_duration = start.elapsed();
        println!("插入 1,000 条记录耗时: {:?}", insert_duration);

        // Search test - use 3+ character query for trigram tokenizer
        let search_start = std::time::Instant::now();
        let results = repository
            .search(
                "测试条目".to_string(),  // Trigram requires 3+ characters
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: false,
                },
                50,
                None,
            )
            .expect("search");
        let search_duration = search_start.elapsed();
        println!("搜索耗时: {:?}, 结果数: {}", search_duration, results.len());

        // Performance assertions
        assert!(insert_duration.as_millis() < 5000, "插入 1,000 条应在 5 秒内完成");
        assert!(search_duration.as_millis() < 100, "搜索应在 100ms 内完成");
        assert!(results.len() >= 50, "应至少返回 50 条结果（trigram 需要 3+ 字符查询）");
    }

    #[test]
    #[ignore] // Long-running test, run with: cargo test -- --ignored
    fn performance_test_10000_items_query() {
        let repository = open_test_repository();

        // Insert 10,000 items
        println!("开始插入 10,000 条记录...");
        let start = std::time::Instant::now();
        for i in 0..10000 {
            repository
                .insert_or_touch(text_draft(&format!("Item {} with content for testing", i)))
                .expect("insert");

            if (i + 1) % 1000 == 0 {
                println!("已插入 {} 条", i + 1);
            }
        }
        let insert_duration = start.elapsed();
        println!("插入完成，耗时: {:?}", insert_duration);

        // Query without search (pagination)
        let query_start = std::time::Instant::now();
        let page1 = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                50,
                None,
            )
            .expect("page 1");
        let query_duration = query_start.elapsed();
        println!("第一页查询耗时: {:?}", query_duration);

        // Query with cursor (second page)
        let cursor_start = std::time::Instant::now();
        let page2 = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                50,
                Some(page1.last().unwrap().updated_at),
            )
            .expect("page 2");
        let cursor_duration = cursor_start.elapsed();
        println!("第二页查询耗时: {:?}", cursor_duration);

        // FTS search
        let fts_start = std::time::Instant::now();
        let search_results = repository
            .search(
                "testing".to_string(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                50,
                None,
            )
            .expect("fts search");
        let fts_duration = fts_start.elapsed();
        println!("FTS 搜索耗时: {:?}, 结果数: {}", fts_duration, search_results.len());

        // Performance assertions
        assert!(query_duration.as_millis() < 50, "分页查询应在 50ms 内完成");
        assert!(cursor_duration.as_millis() < 50, "游标查询应在 50ms 内完成");
        assert!(fts_duration.as_millis() < 100, "FTS 搜索应在 100ms 内完成");
        assert_eq!(page1.len(), 50, "应返回 50 条结果");
        assert_eq!(page2.len(), 50, "第二页应返回 50 条结果");
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFilters {
    pub item_type: Option<String>,
    pub favorites_only: bool,
}

pub struct ClipboardRepository {
    conn: Connection,
}

impl ClipboardRepository {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(INIT_SQL)?;
        migrate_schema(&conn)?;
        conn.execute_batch(INDEX_SQL)?;
        Ok(Self { conn })
    }

    pub fn insert_or_touch(&self, draft: ClipboardItemDraft) -> anyhow::Result<ClipboardItem> {
        let hash = draft.stable_hash();
        let now = Utc::now().timestamp_micros();

        if let Some(existing_id) = self
            .conn
            .query_row(
                "SELECT id FROM clipboard_items WHERE hash = ?1 AND deleted_at IS NULL",
                params![hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            self.conn.execute(
                "UPDATE clipboard_items SET updated_at = ?1, sort_rank = ?1 WHERE id = ?2",
                params![now, existing_id],
            )?;
            return self
                .get_item(&existing_id)?
                .ok_or_else(|| anyhow::anyhow!("item missing"));
        }

        let id = Uuid::new_v4().to_string();
        let item_type = format!("{:?}", draft.item_type).to_lowercase();
        self.conn.execute(
            "INSERT INTO clipboard_items
            (id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, sort_rank, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?9, ?10)",
            params![
                id,
                hash,
                item_type,
                draft.content,
                draft.content_path,
                draft.preview,
                draft.source_app,
                draft.size_bytes,
                now,
                now
            ],
        )?;
        self.rebuild_fts_for_item(&id)?;
        self.get_item(&id)?
            .ok_or_else(|| anyhow::anyhow!("inserted item missing"))
    }

    pub fn reorder_items(&self, ids: &[String]) -> anyhow::Result<()> {
        let now = Utc::now().timestamp_micros();
        for (index, id) in ids.iter().enumerate() {
            let rank = now.saturating_sub(index as i64);
            self.conn.execute(
                "UPDATE clipboard_items SET sort_rank = ?1 WHERE id = ?2 AND deleted_at IS NULL",
                params![rank, id],
            )?;
        }
        Ok(())
    }

    pub fn search(
        &self,
        query: String,
        filters: SearchFilters,
        limit: i64,
        cursor: Option<i64>,
    ) -> anyhow::Result<Vec<ClipboardItem>> {
        let mut sql = String::from(
            "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at
             FROM clipboard_items
             WHERE deleted_at IS NULL",
        );

        if filters.favorites_only {
            sql.push_str(" AND favorite = 1");
        }
        let mut sql_params = Vec::new();

        if let Some(item_type) = filters.item_type {
            sql.push_str(" AND item_type = ?");
            sql_params.push(Value::Text(item_type));
        }
        if let Some(cursor) = cursor {
            sql.push_str(" AND updated_at < ?");
            sql_params.push(Value::Integer(cursor));
        }

        // Hybrid search strategy:
        // - For queries < 3 chars: use LIKE (trigram can't match short strings)
        // - For queries >= 3 chars: use FTS5 trigram (faster, supports CJK)
        if !query.trim().is_empty() {
            let trimmed = query.trim();
            let char_count = trimmed.chars().count();

            if char_count < 3 {
                // Short query: use LIKE for substring matching
                sql.push_str(" AND (preview LIKE ? OR COALESCE(content, '') LIKE ?)");
                let like_pattern = format!("%{}%", trimmed);
                sql_params.push(Value::Text(like_pattern.clone()));
                sql_params.push(Value::Text(like_pattern));
            } else {
                // Long query: use FTS5 trigram
                let fts_query = to_fts_query(trimmed);
                // Only add FTS clause if sanitization produced a valid query
                if !fts_query.is_empty() {
                    sql.push_str(
                        " AND id IN (SELECT id FROM clipboard_items_fts WHERE clipboard_items_fts MATCH ?)",
                    );
                    sql_params.push(Value::Text(fts_query));
                }
                // If sanitization removed everything, fall back to matching all items
            }
        }

        sql.push_str(
            " ORDER BY pinned DESC, COALESCE(sort_rank, updated_at) DESC, updated_at DESC LIMIT ?",
        );
        sql_params.push(Value::Integer(limit));

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(sql_params), Self::map_item)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_item(&self, id: &str) -> anyhow::Result<Option<ClipboardItem>> {
        self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at
                 FROM clipboard_items
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id],
                Self::map_item,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn toggle_favorite(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items SET favorite = CASE favorite WHEN 1 THEN 0 ELSE 1 END WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn toggle_pin(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items SET pinned = CASE pinned WHEN 1 THEN 0 ELSE 1 END WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    pub fn soft_delete(&self, id: &str) -> anyhow::Result<()> {
        let blob_paths = self.content_paths_for_ids(&[id.to_string()])?;
        self.conn.execute(
            "UPDATE clipboard_items SET deleted_at = ?1 WHERE id = ?2",
            params![Utc::now().timestamp_micros(), id],
        )?;
        self.remove_deleted_from_fts()?;
        remove_blob_paths(&blob_paths)?;
        Ok(())
    }

    pub fn prune_history(&self, max_history_items: i64, retention_days: i64) -> anyhow::Result<()> {
        let now = Utc::now().timestamp_micros();

        if retention_days > 0 {
            let cutoff = now - retention_days.saturating_mul(24 * 60 * 60 * 1_000_000);
            let blob_paths = self.content_paths_for_retention_cutoff(cutoff)?;
            self.conn.execute(
                "UPDATE clipboard_items
                 SET deleted_at = ?1
                 WHERE deleted_at IS NULL
                   AND favorite = 0
                   AND updated_at < ?2",
                params![now, cutoff],
            )?;
            remove_blob_paths(&blob_paths)?;
        }

        if max_history_items > 0 {
            let blob_paths = self.content_paths_over_limit(max_history_items)?;
            self.conn.execute(
                "UPDATE clipboard_items
                 SET deleted_at = ?1
                 WHERE deleted_at IS NULL
                   AND favorite = 0
                   AND id IN (
                     SELECT id FROM (
                       SELECT id
                       FROM clipboard_items
                       WHERE deleted_at IS NULL AND favorite = 0
                       ORDER BY updated_at DESC
                       LIMIT -1 OFFSET ?2
                     )
                   )",
                params![now, max_history_items],
            )?;
            remove_blob_paths(&blob_paths)?;
        }

        self.remove_deleted_from_fts()?;
        Ok(())
    }

    pub fn clear_history(&self) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM clipboard_items_fts", [])?;
        self.conn.execute("DELETE FROM clipboard_items", [])?;
        Ok(())
    }

    pub fn find_by_hash(&self, hash: &str) -> anyhow::Result<Option<ClipboardItem>> {
        let item = self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at
                 FROM clipboard_items WHERE hash = ?1 AND deleted_at IS NULL",
                params![hash],
                |row| {
                    Ok(ClipboardItem {
                        id: row.get(0)?,
                        hash: row.get(1)?,
                        item_type: row.get(2)?,
                        content: row.get(3)?,
                        content_path: row.get(4)?,
                        preview: row.get(5)?,
                        source_app: row.get(6)?,
                        favorite: row.get::<_, i64>(7)? == 1,
                        pinned: row.get::<_, i64>(8)? == 1,
                        size_bytes: row.get(9)?,
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                    })
                },
            )
            .optional()?;
        Ok(item)
    }

    pub fn insert_imported_item(&self, item: &ClipboardItem) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO clipboard_items (id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                item.id,
                item.hash,
                item.item_type,
                item.content,
                item.content_path,
                item.preview,
                item.source_app,
                item.favorite,
                item.pinned,
                item.size_bytes,
                item.created_at,
                item.updated_at,
            ],
        )?;

        // 更新 FTS 索引
        self.rebuild_fts_for_item(&item.id)?;

        Ok(())
    }

    fn rebuild_fts_for_item(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM clipboard_items_fts WHERE id = ?1", params![id])?;
        self.conn.execute(
            "INSERT INTO clipboard_items_fts(id, preview, content)
             SELECT id, preview, COALESCE(content, '') FROM clipboard_items WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    }

    fn remove_deleted_from_fts(&self) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM clipboard_items_fts
             WHERE id IN (SELECT id FROM clipboard_items WHERE deleted_at IS NOT NULL)",
            [],
        )?;
        Ok(())
    }

    fn content_paths_for_ids(&self, ids: &[String]) -> anyhow::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for id in ids {
            if let Some(path) = self
                .conn
                .query_row(
                    "SELECT content_path FROM clipboard_items WHERE id = ?1 AND deleted_at IS NULL",
                    params![id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten()
            {
                paths.push(PathBuf::from(path));
            }
        }
        Ok(paths)
    }

    fn content_paths_for_retention_cutoff(&self, cutoff: i64) -> anyhow::Result<Vec<PathBuf>> {
        let mut statement = self.conn.prepare(
            "SELECT content_path
             FROM clipboard_items
             WHERE deleted_at IS NULL
               AND favorite = 0
               AND updated_at < ?1
               AND content_path IS NOT NULL",
        )?;
        let rows = statement.query_map(params![cutoff], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn content_paths_over_limit(&self, max_history_items: i64) -> anyhow::Result<Vec<PathBuf>> {
        let mut statement = self.conn.prepare(
            "SELECT content_path
             FROM clipboard_items
             WHERE deleted_at IS NULL
               AND favorite = 0
               AND content_path IS NOT NULL
               AND id IN (
                 SELECT id FROM (
                   SELECT id
                   FROM clipboard_items
                   WHERE deleted_at IS NULL AND favorite = 0
                   ORDER BY updated_at DESC
                   LIMIT -1 OFFSET ?1
                 )
               )",
        )?;
        let rows =
            statement.query_map(params![max_history_items], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    fn map_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardItem> {
        Ok(ClipboardItem {
            id: row.get(0)?,
            hash: row.get(1)?,
            item_type: row.get(2)?,
            content: row.get(3)?,
            content_path: row.get(4)?,
            preview: row.get(5)?,
            source_app: row.get(6)?,
            favorite: row.get::<_, i64>(7)? == 1,
            pinned: row.get::<_, i64>(8)? == 1,
            size_bytes: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
        })
    }
}

fn to_fts_query(raw: &str) -> String {
    // Security: Strict sanitization to prevent FTS5 injection attacks
    // With trigram tokenizer, we search for substrings directly
    raw.split_whitespace()
        .filter_map(|token| {
            // Only allow alphanumeric characters, basic punctuation, and CJK characters
            let cleaned: String = token
                .chars()
                .filter(|c| {
                    c.is_alphanumeric()
                        || matches!(c, '-' | '_' | '.' | '@')
                        || (*c >= '\u{4E00}' && *c <= '\u{9FFF}') // CJK Unified Ideographs
                        || (*c >= '\u{3400}' && *c <= '\u{4DBF}') // CJK Extension A
                        || (*c >= '\u{20000}' && *c <= '\u{2A6DF}') // CJK Extension B
                        || (*c >= '\u{AC00}' && *c <= '\u{D7AF}') // Hangul
                        || (*c >= '\u{3040}' && *c <= '\u{309F}') // Hiragana
                        || (*c >= '\u{30A0}' && *c <= '\u{30FF}') // Katakana
                })
                .collect();

            if cleaned.is_empty() {
                None
            } else {
                // Escape quotes and wrap in quotes for exact phrase matching
                // This prevents FTS5 operator injection (AND, OR, NOT, etc.)
                Some(format!("\"{}\"", cleaned.replace('"', "\"\"")))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn migrate_schema(conn: &Connection) -> anyhow::Result<()> {
    let columns = conn
        .prepare("PRAGMA table_info(clipboard_items)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;

    if !columns.iter().any(|name| name == "pinned") {
        conn.execute(
            "ALTER TABLE clipboard_items ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }

    if !columns.iter().any(|name| name == "sort_rank") {
        conn.execute(
            "ALTER TABLE clipboard_items ADD COLUMN sort_rank INTEGER",
            [],
        )?;
    }

    conn.execute(
        "UPDATE clipboard_items SET sort_rank = updated_at WHERE sort_rank IS NULL",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_clipboard_items_sort_rank ON clipboard_items(sort_rank DESC, updated_at DESC)",
        [],
    )?;

    // Migrate FTS5 table to support CJK substring search with trigram tokenizer
    // Check if the FTS5 table needs to be rebuilt with trigram tokenizer
    let needs_fts_rebuild = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='clipboard_items_fts'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .map(|sql| !sql.contains("trigram"))
        .unwrap_or(false);

    if needs_fts_rebuild {
        crate::diagnostics::info("Migrating FTS5 table to use trigram tokenizer for CJK support");

        // Drop old FTS5 table
        conn.execute("DROP TABLE IF EXISTS clipboard_items_fts", [])?;

        // Create new FTS5 table with trigram tokenizer
        conn.execute(
            "CREATE VIRTUAL TABLE clipboard_items_fts USING fts5(id UNINDEXED, preview, content, tokenize='trigram')",
            [],
        )?;

        // Rebuild FTS index from existing items
        conn.execute(
            "INSERT INTO clipboard_items_fts(id, preview, content)
             SELECT id, preview, content FROM clipboard_items WHERE deleted_at IS NULL",
            [],
        )?;

        crate::diagnostics::info("FTS5 table migration completed");
    }

    Ok(())
}

fn remove_blob_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    for path in paths {
        crate::blobs::remove_blob_if_exists(path)?;
        let thumbnail_path = crate::blobs::thumbnail_path_for(Path::new(path));
        crate::blobs::remove_blob_if_exists(&thumbnail_path)?;
    }
    Ok(())
}
