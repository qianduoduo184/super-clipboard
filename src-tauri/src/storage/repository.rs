use std::path::PathBuf;

use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clipboard::types::ClipboardItemDraft;

use super::schema::INIT_SQL;

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

        let first = repository.insert_or_touch(text_draft("hello")).expect("first insert");
        let second = repository.insert_or_touch(text_draft("hello")).expect("second insert");

        assert_eq!(first.id, second.id);
    }

    #[test]
    fn search_returns_matching_items() {
        let repository = open_test_repository();
        repository.insert_or_touch(text_draft("sqlite clipboard history")).expect("insert");

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
    fn insert_or_touch_allows_reinserting_soft_deleted_content() {
        let repository = open_test_repository();

        let first = repository.insert_or_touch(text_draft("repeatable")).expect("first insert");
        repository.soft_delete(&first.id).expect("soft delete");
        let second = repository.insert_or_touch(text_draft("repeatable")).expect("second insert");

        assert_ne!(first.id, second.id);
    }

    #[test]
    fn prune_history_soft_deletes_old_non_favorites_over_limit() {
        let repository = open_test_repository();
        let first = repository.insert_or_touch(text_draft("first")).expect("first insert");
        let second = repository.insert_or_touch(text_draft("second")).expect("second insert");
        let favorite = repository.insert_or_touch(text_draft("favorite")).expect("favorite insert");
        repository.toggle_favorite(&favorite.id).expect("favorite");

        repository.prune_history(1, 0).expect("prune");

        let first_active = repository.get_item(&first.id).expect("first").is_some();
        let second_active = repository.get_item(&second.id).expect("second").is_some();
        assert_ne!(first_active, second_active);
        assert!(repository.get_item(&favorite.id).expect("favorite").is_some());
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
        Ok(Self { conn })
    }

    pub fn insert_or_touch(&self, draft: ClipboardItemDraft) -> anyhow::Result<ClipboardItem> {
        let hash = draft.stable_hash();
        let now = Utc::now().timestamp_millis();

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
                "UPDATE clipboard_items SET updated_at = ?1 WHERE id = ?2",
                params![now, existing_id],
            )?;
            return self.get_item(&existing_id)?.ok_or_else(|| anyhow::anyhow!("item missing"));
        }

        let id = Uuid::new_v4().to_string();
        let item_type = format!("{:?}", draft.item_type).to_lowercase();
        self.conn.execute(
            "INSERT INTO clipboard_items
            (id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, created_at, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10)",
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
        self.get_item(&id)?.ok_or_else(|| anyhow::anyhow!("inserted item missing"))
    }

    pub fn search(
        &self,
        query: String,
        filters: SearchFilters,
        limit: i64,
        cursor: Option<i64>,
    ) -> anyhow::Result<Vec<ClipboardItem>> {
        let mut sql = String::from(
            "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, created_at, updated_at
             FROM clipboard_items
             WHERE deleted_at IS NULL",
        );

        if filters.favorites_only {
            sql.push_str(" AND favorite = 1");
        }
        let mut sql_params = Vec::new();

        if filters.item_type.is_some() {
            sql.push_str(" AND item_type = :item_type");
            sql_params.push(Value::Text(filters.item_type.unwrap_or_default()));
        }
        if cursor.is_some() {
            sql.push_str(" AND updated_at < :cursor");
            sql_params.push(Value::Integer(cursor.unwrap_or(i64::MAX)));
        }
        if !query.trim().is_empty() {
            sql.push_str(
                " AND id IN (SELECT id FROM clipboard_items_fts WHERE clipboard_items_fts MATCH :query)",
            );
            sql_params.push(Value::Text(query));
        }
        sql.push_str(" ORDER BY updated_at DESC LIMIT :limit");
        sql_params.push(Value::Integer(limit));

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(sql_params), Self::map_item)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_item(&self, id: &str) -> anyhow::Result<Option<ClipboardItem>> {
        self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, created_at, updated_at
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

    pub fn soft_delete(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE clipboard_items SET deleted_at = ?1 WHERE id = ?2",
            params![Utc::now().timestamp_millis(), id],
        )?;
        self.remove_deleted_from_fts()?;
        Ok(())
    }

    pub fn prune_history(&self, max_history_items: i64, retention_days: i64) -> anyhow::Result<()> {
        let now = Utc::now().timestamp_millis();

        if retention_days > 0 {
            let cutoff = now - retention_days.saturating_mul(24 * 60 * 60 * 1000);
            self.conn.execute(
                "UPDATE clipboard_items
                 SET deleted_at = ?1
                 WHERE deleted_at IS NULL
                   AND favorite = 0
                   AND updated_at < ?2",
                params![now, cutoff],
            )?;
        }

        if max_history_items > 0 {
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
        }

        self.remove_deleted_from_fts()?;
        Ok(())
    }

    pub fn clear_history(&self) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM clipboard_items_fts", [])?;
        self.conn.execute("DELETE FROM clipboard_items", [])?;
        Ok(())
    }

    fn rebuild_fts_for_item(&self, id: &str) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM clipboard_items_fts WHERE id = ?1", params![id])?;
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
            size_bytes: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }
}
