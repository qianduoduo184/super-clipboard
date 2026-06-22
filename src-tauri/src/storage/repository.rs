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

        let url_results = repository
            .search(
                "http://example.com/path".to_string(),
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

        assert_eq!(url_results.len(), 1);
        assert!(punctuation_results.is_empty());
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
        if !query.trim().is_empty() {
            sql.push_str(
                " AND id IN (SELECT id FROM clipboard_items_fts WHERE clipboard_items_fts MATCH ?)",
            );
            sql_params.push(Value::Text(to_fts_query(&query)));
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
    raw.split_whitespace()
        .map(|token| {
            // Escape double quotes and strip FTS5 special characters
            let escaped = token.replace('"', "\"\"");
            // Remove FTS5 operators that could break the query
            let cleaned = escaped
                .chars()
                .filter(|c| !matches!(c, '*' | '^' | '(' | ')' | ':'))
                .collect::<String>();
            if cleaned.is_empty() {
                String::new()
            } else {
                // Use prefix match for better substring matching (especially for CJK)
                format!("\"{}\"*", cleaned)
            }
        })
        .filter(|s| !s.is_empty())
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
