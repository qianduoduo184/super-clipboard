use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::{params, params_from_iter, types::Value, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};

use super::schema::{INDEX_SQL, INIT_SQL};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItem {
    pub id: String,
    pub hash: String,
    pub item_type: String,
    pub content: Option<String>,
    pub content_path: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub preview: String,
    pub source_app: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItemSummary {
    pub id: String,
    pub hash: String,
    pub item_type: String,
    pub preview: String,
    pub source_app: Option<String>,
    pub favorite: bool,
    pub pinned: bool,
    pub size_bytes: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub content_path: Option<String>,
    pub thumbnail_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchCursor {
    pub pinned: bool,
    pub effective_rank: i64,
    pub updated_at: i64,
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClipboardSearchPage {
    pub items: Vec<ClipboardItemSummary>,
    pub next_cursor: Option<SearchCursor>,
}

impl std::ops::Deref for ClipboardSearchPage {
    type Target = [ClipboardItemSummary];

    fn deref(&self) -> &Self::Target {
        &self.items
    }
}

impl IntoIterator for ClipboardSearchPage {
    type Item = ClipboardItemSummary;
    type IntoIter = std::vec::IntoIter<ClipboardItemSummary>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
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
            content_hash: None,
            preview: content.to_string(),
            source_app: Some("test".to_string()),
            size_bytes: content.len() as i64,
        }
    }

    fn image_draft(content_hash: &str, content_path: &str) -> ClipboardItemDraft {
        ClipboardItemDraft {
            item_type: ClipboardItemType::Image,
            content: None,
            content_path: Some(content_path.to_string()),
            content_hash: Some(content_hash.to_string()),
            preview: "image".to_string(),
            source_app: Some("test".to_string()),
            size_bytes: 128,
        }
    }

    #[test]
    fn open_migrates_legacy_database_with_nullable_content_hash() {
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
        let content_hash: Option<String> = repository
            .conn
            .query_row(
                "SELECT content_hash FROM clipboard_items WHERE id = 'legacy-id'",
                [],
                |row| row.get(0),
            )
            .expect("legacy content hash");

        assert_eq!(content_hash, None);
    }

    #[test]
    fn schema_enforces_active_image_uniqueness_and_allows_deleted_duplicate() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch_image(image_draft("same-pixels", "first.bmp"))
            .expect("first image");

        let duplicate = repository.conn.execute(
            "INSERT INTO clipboard_items
             (id, hash, item_type, content_path, content_hash, preview, favorite, pinned, size_bytes, created_at, updated_at)
             VALUES ('duplicate', 'different-hash', 'image', 'second.bmp', 'same-pixels', 'image', 0, 0, 128, 2, 2)",
            [],
        );
        assert!(duplicate.is_err());

        repository.soft_delete(&first.id).expect("soft delete");
        let replacement = repository
            .insert_or_touch_image(image_draft("same-pixels", "second.bmp"))
            .expect("replacement image");
        assert_ne!(first.id, replacement.id);
    }

    #[test]
    fn schema_creates_migration_state_and_cleanup_queue() {
        let repository = open_test_repository();
        let migration_columns = repository
            .conn
            .prepare("PRAGMA table_info(schema_migrations)")
            .expect("migration schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("migration columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("migration column names");
        let cleanup_columns = repository
            .conn
            .prepare("PRAGMA table_info(blob_cleanup_queue)")
            .expect("cleanup schema")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("cleanup columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("cleanup column names");

        assert_eq!(
            migration_columns,
            vec!["name", "state", "backup_path", "created_at", "updated_at"]
        );
        assert_eq!(cleanup_columns, vec!["path", "created_at"]);
    }

    #[test]
    fn image_repository_uses_content_identity_and_lists_active_paths() {
        let repository = open_test_repository();
        let inserted = repository
            .insert_or_touch_image(image_draft("pixel-hash", "canonical.bmp"))
            .expect("insert image");
        let touched = repository
            .insert_or_touch_image(image_draft("pixel-hash", "ignored.bmp"))
            .expect("touch image");

        assert_eq!(inserted.id, touched.id);
        assert_eq!(inserted.hash, "image:pixel-hash");
        assert_eq!(inserted.content_hash.as_deref(), Some("pixel-hash"));
        assert_eq!(
            repository
                .find_active_image("pixel-hash")
                .expect("find image")
                .expect("active image")
                .id,
            inserted.id
        );
        assert_eq!(
            repository.active_blob_paths().expect("active paths"),
            vec![PathBuf::from("canonical.bmp")]
        );
    }

    #[test]
    fn insert_or_touch_image_rolls_back_insert_when_fts_rebuild_fails() {
        let repository = open_test_repository();
        repository
            .conn
            .execute("DROP TABLE clipboard_items_fts", [])
            .expect("drop FTS table");

        let result = repository.insert_or_touch_image(image_draft("pixel-hash", "image.bmp"));
        let active_images = repository
            .conn
            .query_row(
                "SELECT COUNT(*) FROM clipboard_items
                 WHERE item_type = 'image' AND content_hash = ?1 AND deleted_at IS NULL",
                params!["pixel-hash"],
                |row| row.get::<_, i64>(0),
            )
            .expect("count active images");

        assert!(result.is_err());
        assert_eq!(active_images, 0);
    }

    #[test]
    fn insert_or_touch_image_rolls_back_touch_when_fts_rebuild_fails() {
        let repository = open_test_repository();
        let image = repository
            .insert_or_touch_image(image_draft("pixel-hash", "image.bmp"))
            .expect("insert image");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items SET updated_at = 1, sort_rank = 1 WHERE id = ?1",
                params![image.id],
            )
            .expect("set stable recency");
        repository
            .conn
            .execute("DROP TABLE clipboard_items_fts", [])
            .expect("drop FTS table");

        let result = repository.insert_or_touch_image(image_draft("pixel-hash", "ignored.bmp"));
        let recency = repository
            .conn
            .query_row(
                "SELECT updated_at, sort_rank FROM clipboard_items WHERE id = ?1",
                params![image.id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .expect("load recency");

        assert!(result.is_err());
        assert_eq!(recency, (1, 1));
    }

    #[test]
    fn active_blob_paths_includes_legacy_image_without_content_hash() {
        let repository = open_test_repository();
        repository
            .conn
            .execute(
                "INSERT INTO clipboard_items
                 (id, hash, item_type, content_path, content_hash, preview, favorite, pinned, size_bytes, created_at, updated_at)
                 VALUES ('legacy-image', 'legacy-image-hash', 'image', 'legacy.bmp', NULL, 'legacy image', 0, 0, 128, 1, 1)",
                [],
            )
            .expect("insert legacy image");

        assert_eq!(
            repository.active_blob_paths().expect("active paths"),
            vec![PathBuf::from("legacy.bmp")]
        );
    }

    #[test]
    fn cleanup_queue_can_list_and_complete_paths() {
        let repository = open_test_repository();
        let image = repository
            .insert_or_touch_image(image_draft("pixel-hash", "obsolete.bmp"))
            .expect("insert image");
        repository.soft_delete(&image.id).expect("soft delete");

        assert_eq!(
            repository.pending_cleanup_paths().expect("pending cleanup"),
            vec![
                PathBuf::from("obsolete.bmp"),
                crate::blobs::thumbnail_path_for(Path::new("obsolete.bmp")),
            ]
        );
        for path in [
            PathBuf::from("obsolete.bmp"),
            crate::blobs::thumbnail_path_for(Path::new("obsolete.bmp")),
        ] {
            repository
                .complete_cleanup_path(&path)
                .expect("complete cleanup");
        }
        assert!(repository
            .pending_cleanup_paths()
            .expect("completed cleanup")
            .is_empty());
    }

    #[test]
    fn soft_delete_rolls_back_when_thumbnail_enqueue_fails() {
        let repository = open_test_repository();
        let image = repository
            .insert_or_touch_image(image_draft("pixel-hash", "rollback.bmp"))
            .expect("insert image");
        repository
            .conn
            .execute_batch(
                "CREATE TRIGGER fail_thumbnail_cleanup
                 BEFORE INSERT ON blob_cleanup_queue
                 WHEN NEW.path LIKE '%.thumb.png'
                 BEGIN
                   SELECT RAISE(ABORT, 'thumbnail queue failure');
                 END;",
            )
            .expect("create cleanup failure trigger");

        let result = repository.soft_delete(&image.id);

        assert!(result.is_err());
        assert!(repository
            .get_item(&image.id)
            .expect("image after rollback")
            .is_some());
        assert!(repository
            .pending_cleanup_paths()
            .expect("cleanup rollback")
            .is_empty());
    }

    #[test]
    fn capacity_batch_soft_delete_rolls_back_rows_fts_and_cleanup_queue_together() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch_image(image_draft("batch-first", "C:/blobs/batch-first.bmp"))
            .expect("first image");
        let second = repository
            .insert_or_touch_image(image_draft("batch-second", "C:/blobs/batch-second.bmp"))
            .expect("second image");
        repository
            .conn
            .execute_batch(&format!(
                "CREATE TRIGGER fail_capacity_batch
                 BEFORE UPDATE OF deleted_at ON clipboard_items
                 WHEN OLD.id = '{}'
                 BEGIN
                   SELECT RAISE(ABORT, 'injected capacity batch failure');
                 END;",
                second.id
            ))
            .expect("failure trigger");

        repository
            .soft_delete_batch(&[first.id.clone(), second.id.clone()])
            .expect_err("batch must roll back");

        assert!(repository
            .get_item(&first.id)
            .expect("first query")
            .is_some());
        assert!(repository
            .get_item(&second.id)
            .expect("second query")
            .is_some());
        assert!(repository
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        let fts_count: i64 = repository
            .conn
            .query_row(
                "SELECT COUNT(*) FROM clipboard_items_fts WHERE id IN (?1, ?2)",
                params![first.id, second.id],
                |row| row.get(0),
            )
            .expect("fts count");
        assert_eq!(fts_count, 2);
    }

    #[test]
    fn prune_history_enqueues_image_and_thumbnail_paths() {
        let repository = open_test_repository();
        let old = repository
            .insert_or_touch_image(image_draft("old-hash", "old.bmp"))
            .expect("old image");
        let recent = repository
            .insert_or_touch_image(image_draft("recent-hash", "recent.bmp"))
            .expect("recent image");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items SET updated_at = 1 WHERE id = ?1",
                params![old.id],
            )
            .expect("age old image");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items SET updated_at = 2 WHERE id = ?1",
                params![recent.id],
            )
            .expect("keep recent image");

        repository.prune_history(1, 0).expect("prune history");

        assert_eq!(
            repository.pending_cleanup_paths().expect("pending cleanup"),
            vec![
                PathBuf::from("old.bmp"),
                crate::blobs::thumbnail_path_for(Path::new("old.bmp")),
            ]
        );
        assert!(repository.get_item(&old.id).expect("old image").is_none());
        assert!(repository
            .get_item(&recent.id)
            .expect("recent image")
            .is_some());
    }

    #[test]
    fn clear_history_enqueues_image_and_thumbnail_paths() {
        let repository = open_test_repository();
        repository
            .insert_or_touch_image(image_draft("pixel-hash", "clear.bmp"))
            .expect("insert image");

        repository.clear_history().expect("clear history");

        assert_eq!(
            repository.pending_cleanup_paths().expect("pending cleanup"),
            vec![
                PathBuf::from("clear.bmp"),
                crate::blobs::thumbnail_path_for(Path::new("clear.bmp")),
            ]
        );
    }

    #[test]
    fn reference_updates_and_cleanup_enqueue_roll_back_together() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch_image(image_draft("first-hash", "first.bmp"))
            .expect("first image");
        let second = repository
            .insert_or_touch_image(image_draft("second-hash", "second.bmp"))
            .expect("second image");
        let updates = vec![
            ImageReferenceUpdate {
                item_id: first.id.clone(),
                content_hash: "merged-hash".to_string(),
                content_path: PathBuf::from("merged.bmp"),
            },
            ImageReferenceUpdate {
                item_id: second.id.clone(),
                content_hash: "merged-hash".to_string(),
                content_path: PathBuf::from("merged.bmp"),
            },
        ];

        let result = repository.update_image_references_and_enqueue_cleanup(
            &updates,
            &[PathBuf::from("first.bmp"), PathBuf::from("second.bmp")],
        );

        assert!(result.is_err());
        assert_eq!(
            repository
                .get_item(&first.id)
                .expect("first item")
                .expect("active first")
                .content_hash
                .as_deref(),
            Some("first-hash")
        );
        assert!(repository
            .pending_cleanup_paths()
            .expect("cleanup rollback")
            .is_empty());
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
    fn lightweight_summary_search_serialization_omits_full_content_fields() {
        let repository = open_test_repository();
        repository
            .insert_or_touch(text_draft("large clipboard payload"))
            .expect("insert");

        let summary = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("summary search")
            .into_iter()
            .next()
            .expect("summary");
        let json = serde_json::to_value(summary).expect("serialize summary");
        let object = json.as_object().expect("summary object");

        assert!(!object.contains_key("content"));
        assert!(!object.contains_key("content_hash"));
    }

    #[test]
    fn lightweight_summary_projection_does_not_read_content() {
        let repository = open_test_repository();
        repository
            .conn
            .execute(
                "INSERT INTO clipboard_items
                 (id, hash, item_type, content, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, sort_rank)
                 VALUES ('poison-content', 'poison-hash', 'text', X'80FF', 'safe preview', 'fixture', 0, 0, 2, 10, 20, 20)",
                [],
            )
            .expect("insert non-text content fixture");

        let summaries = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("summary projection must not decode content");

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].preview, "safe preview");
    }

    #[test]
    fn lightweight_summary_derives_only_image_thumbnail_path() {
        let repository = open_test_repository();
        let image = repository
            .insert_or_touch_image(image_draft("pixels", "legacy/images/original.bmp"))
            .expect("insert image");
        let text = repository
            .insert_or_touch(text_draft("plain text"))
            .expect("insert text");

        let summaries = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            )
            .expect("summary search");
        let image_json = summaries
            .iter()
            .find(|item| item.id == image.id)
            .map(serde_json::to_value)
            .expect("image summary")
            .expect("serialize image summary");
        let text_json = summaries
            .iter()
            .find(|item| item.id == text.id)
            .map(serde_json::to_value)
            .expect("text summary")
            .expect("serialize text summary");

        assert_eq!(image_json["content_path"], "legacy/images/original.bmp");
        assert_eq!(
            image_json["thumbnail_path"],
            crate::blobs::thumbnail_path_for(Path::new("legacy/images/original.bmp"))
                .to_string_lossy()
                .as_ref()
        );
        assert_eq!(text_json["content_path"], serde_json::Value::Null);
        assert_eq!(text_json["thumbnail_path"], serde_json::Value::Null);
    }

    #[test]
    fn lightweight_summary_get_item_preserves_text_and_html_content() {
        let repository = open_test_repository();
        let text = repository
            .insert_or_touch(text_draft("full text content"))
            .expect("insert text");
        let html = repository
            .insert_or_touch(ClipboardItemDraft {
                item_type: ClipboardItemType::Html,
                content: Some("<p>full <strong>HTML</strong></p>".to_string()),
                content_path: None,
                content_hash: None,
                preview: "full HTML".to_string(),
                source_app: Some("browser".to_string()),
                size_bytes: 33,
            })
            .expect("insert HTML");

        assert_eq!(
            repository
                .get_item(&text.id)
                .expect("get text")
                .expect("text item")
                .content
                .as_deref(),
            Some("full text content")
        );
        assert_eq!(
            repository
                .get_item(&html.id)
                .expect("get HTML")
                .expect("HTML item")
                .content
                .as_deref(),
            Some("<p>full <strong>HTML</strong></p>")
        );
    }

    #[test]
    fn lightweight_summary_preserves_filters_pagination_and_metadata() {
        let repository = open_test_repository();
        let newest = repository
            .insert_or_touch(text_draft("newest favorite"))
            .expect("insert newest");
        let older = repository
            .insert_or_touch(text_draft("older favorite"))
            .expect("insert older");
        let excluded = repository
            .insert_or_touch(text_draft("excluded non-favorite"))
            .expect("insert excluded");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items
                 SET source_app = 'editor', favorite = 1, pinned = 1,
                     size_bytes = 111, created_at = 30, updated_at = 300, sort_rank = 300
                 WHERE id = ?1",
                params![newest.id],
            )
            .expect("update newest metadata");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items
                 SET source_app = 'terminal', favorite = 1, pinned = 0,
                     size_bytes = 222, created_at = 20, updated_at = 200, sort_rank = 200
                 WHERE id = ?1",
                params![older.id],
            )
            .expect("update older metadata");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items
                 SET favorite = 0, created_at = 10, updated_at = 100, sort_rank = 100
                 WHERE id = ?1",
                params![excluded.id],
            )
            .expect("update excluded metadata");

        let first_page = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: true,
                },
                1,
                None,
            )
            .expect("first page");
        let next_cursor = first_page.next_cursor.clone();
        let second_page = repository
            .search(
                String::new(),
                SearchFilters {
                    item_type: Some("text".to_string()),
                    favorites_only: true,
                },
                1,
                next_cursor,
            )
            .expect("second page");

        assert_eq!(first_page.len(), 1);
        assert_eq!(first_page[0].id, newest.id);
        assert_eq!(first_page[0].source_app.as_deref(), Some("editor"));
        assert!(first_page[0].favorite);
        assert!(first_page[0].pinned);
        assert_eq!(first_page[0].size_bytes, 111);
        assert_eq!(first_page[0].created_at, 30);
        assert_eq!(first_page[0].updated_at, 300);
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].id, older.id);
        assert_ne!(second_page[0].id, excluded.id);
    }

    #[test]
    fn search_pagination_follows_pinned_order_before_updated_time() {
        let repository = open_test_repository();
        let pinned = repository
            .insert_or_touch(text_draft("pinned older item"))
            .expect("insert pinned item");
        let recent = repository
            .insert_or_touch(text_draft("recent unpinned item"))
            .expect("insert recent item");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items
                 SET pinned = 1, updated_at = 100, sort_rank = 100
                 WHERE id = ?1",
                params![pinned.id],
            )
            .expect("pin older item");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items
                 SET pinned = 0, updated_at = 300, sort_rank = 300
                 WHERE id = ?1",
                params![recent.id],
            )
            .expect("update recent item");

        let filters = SearchFilters {
            item_type: None,
            favorites_only: false,
        };
        let first_page = repository
            .search(String::new(), filters.clone(), 1, None)
            .expect("first page");
        let next_cursor = first_page.next_cursor.clone();
        let second_page = repository
            .search(String::new(), filters, 1, next_cursor)
            .expect("second page");

        assert_eq!(first_page[0].id, pinned.id);
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page[0].id, recent.id);
    }

    #[test]
    fn search_pagination_follows_manual_sort_rank_without_duplicates() {
        let repository = open_test_repository();
        let first = repository
            .insert_or_touch(text_draft("first by manual rank"))
            .expect("insert first");
        let second = repository
            .insert_or_touch(text_draft("second by manual rank"))
            .expect("insert second");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items SET updated_at = 300, sort_rank = 100 WHERE id = ?1",
                params![first.id],
            )
            .expect("rank first");
        repository
            .conn
            .execute(
                "UPDATE clipboard_items SET updated_at = 100, sort_rank = 300 WHERE id = ?1",
                params![second.id],
            )
            .expect("rank second");

        let filters = SearchFilters {
            item_type: None,
            favorites_only: false,
        };
        let first_page = repository
            .search(String::new(), filters.clone(), 1, None)
            .expect("first page");
        let second_page = repository
            .search(String::new(), filters, 1, first_page.next_cursor.clone())
            .expect("second page");

        assert_eq!(first_page[0].id, second.id);
        assert_eq!(second_page[0].id, first.id);
        assert_ne!(first_page[0].id, second_page[0].id);
    }

    #[test]
    fn duplicate_capture_refreshes_a_known_source() {
        let repository = open_test_repository();
        let mut first_draft = text_draft("same clipboard content");
        first_draft.source_app = None;
        let first = repository
            .insert_or_touch(first_draft)
            .expect("insert first capture");

        let mut repeated_draft = text_draft("same clipboard content");
        repeated_draft.source_app = Some("Code.exe".to_string());
        let repeated = repository
            .insert_or_touch(repeated_draft)
            .expect("touch duplicate capture");

        assert_eq!(repeated.id, first.id);
        assert_eq!(repeated.source_app.as_deref(), Some("Code.exe"));
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
                "example".to_string(), // Search for domain name keyword instead of full URL
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
        )
        .expect("create table");

        conn.execute(
            "INSERT INTO test_fts(content) VALUES (?)",
            ["同步组织排序码到云之家"],
        )
        .expect("insert");

        // First, check if data is actually in the table
        let row_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM test_fts", [], |row| row.get(0))
            .expect("count");
        println!("Total rows in FTS table: {}", row_count);

        // Check the actual content
        let content: String = conn
            .query_row("SELECT content FROM test_fts", [], |row| row.get(0))
            .expect("get content");
        println!("Stored content: {}", content);

        // Test trigram tokenizer with CJK text
        // Trigram creates 3-character tokens, ideal for CJK substring search

        // Test 1: Exact substring match (trigram should handle this)
        let q1 = r#""云之家""#;
        let c1: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
                [q1],
                |row| row.get(0),
            )
            .unwrap_or(0);
        println!("Query '{}' -> {}", q1, c1);

        // Test 2: Single character
        let q2 = r#""云""#;
        let c2: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
                [q2],
                |row| row.get(0),
            )
            .unwrap_or(0);
        println!("Query '{}' -> {}", q2, c2);

        // Test 3: Two characters
        let q3 = r#""云之""#;
        let c3: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM test_fts WHERE test_fts MATCH ?",
                [q3],
                |row| row.get(0),
            )
            .unwrap_or(0);
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
        assert_eq!(
            results_2char.len(),
            2,
            "Should find 'ab' substring in two items (123ab332sddsdf and test ab content)"
        );

        // Verify the specific items found
        let found_previews: Vec<&str> = results_2char
            .iter()
            .map(|item| item.preview.as_str())
            .collect();
        assert!(
            found_previews.contains(&"123ab332sddsdf"),
            "Should find '123ab332sddsdf'"
        );
        assert!(
            found_previews.contains(&"test ab content"),
            "Should find 'test ab content'"
        );

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
        assert_eq!(
            results_3char.len(),
            1,
            "Should find '3ab' in one item via trigram"
        );
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
        assert_eq!(
            results_middle.len(),
            1,
            "Should find 'ab3' substring via trigram"
        );
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
                .insert_or_touch(text_draft(&format!(
                    "测试条目 {}: 这是一段中文内容用于测试搜索性能",
                    i
                )))
                .expect("insert");
        }
        let insert_duration = start.elapsed();
        println!("插入 1,000 条记录耗时: {:?}", insert_duration);

        // Search test - use 3+ character query for trigram tokenizer
        let search_start = std::time::Instant::now();
        let results = repository
            .search(
                "测试条目".to_string(), // Trigram requires 3+ characters
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
        assert!(
            insert_duration.as_millis() < 5000,
            "插入 1,000 条应在 5 秒内完成"
        );
        assert!(search_duration.as_millis() < 100, "搜索应在 100ms 内完成");
        assert!(
            results.len() >= 50,
            "应至少返回 50 条结果（trigram 需要 3+ 字符查询）"
        );
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
        let next_cursor = page1.next_cursor.clone();
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
                next_cursor,
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
        println!(
            "FTS 搜索耗时: {:?}, 结果数: {}",
            fts_duration,
            search_results.len()
        );

        // Performance assertions
        assert!(query_duration.as_millis() < 50, "分页查询应在 50ms 内完成");
        assert!(cursor_duration.as_millis() < 50, "游标查询应在 50ms 内完成");
        assert!(fts_duration.as_millis() < 100, "FTS 搜索应在 100ms 内完成");
        assert_eq!(page1.len(), 50, "应返回 50 条结果");
        assert_eq!(page2.len(), 50, "第二页应返回 50 条结果");
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReferenceUpdate {
    pub item_id: String,
    pub content_hash: String,
    pub content_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationRecord {
    pub state: String,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MigrationImageRow {
    pub id: String,
    pub content_path: Option<PathBuf>,
    pub favorite: bool,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub sort_rank: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImageMigrationMerge {
    pub retained_id: String,
    pub duplicate_ids: Vec<String>,
    pub content_hash: String,
    pub content_path: PathBuf,
    pub favorite: bool,
    pub pinned: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub sort_rank: i64,
    pub size_bytes: i64,
    pub obsolete_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchFilters {
    pub item_type: Option<String>,
    pub favorites_only: bool,
}

pub struct ClipboardRepository {
    conn: Connection,
    database_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum RepositoryImportMode {
    Merge,
    Overwrite,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RepositoryImportResult {
    pub imported: usize,
    pub skipped: usize,
}

impl ClipboardRepository {
    pub fn open(path: PathBuf) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch(INIT_SQL)?;
        migrate_schema(&conn)?;
        conn.execute_batch(INDEX_SQL)?;
        Ok(Self {
            conn,
            database_path: path,
        })
    }

    pub(crate) fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub(crate) fn checkpoint_wal(&self) -> anyhow::Result<()> {
        let busy = self
            .conn
            .query_row("PRAGMA wal_checkpoint(FULL)", [], |row| {
                row.get::<_, i64>(0)
            })?;
        anyhow::ensure!(busy == 0, "SQLite WAL checkpoint remained busy");
        Ok(())
    }

    pub(crate) fn migration_record(&self, name: &str) -> anyhow::Result<Option<MigrationRecord>> {
        self.conn
            .query_row(
                "SELECT state, backup_path FROM schema_migrations WHERE name = ?1",
                params![name],
                |row| {
                    Ok(MigrationRecord {
                        state: row.get(0)?,
                        backup_path: row.get::<_, Option<String>>(1)?.map(PathBuf::from),
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn reserve_migration_backup(
        &self,
        name: &str,
        backup_path: &Path,
    ) -> anyhow::Result<MigrationRecord> {
        let now = Utc::now().timestamp_micros();
        self.conn.execute(
            "INSERT OR IGNORE INTO schema_migrations
             (name, state, backup_path, created_at, updated_at)
             VALUES (?1, 'pending', ?2, ?3, ?3)",
            params![name, backup_path.to_string_lossy(), now],
        )?;
        self.migration_record(name)?
            .ok_or_else(|| anyhow::anyhow!("reserved migration row is missing"))
    }

    pub(crate) fn set_migration_state(&self, name: &str, state: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(state, "pending" | "cleanup_pending" | "complete"),
            "invalid migration state"
        );
        let changed = self.conn.execute(
            "UPDATE schema_migrations SET state = ?1, updated_at = ?2 WHERE name = ?3",
            params![state, Utc::now().timestamp_micros(), name],
        )?;
        anyhow::ensure!(changed == 1, "migration row is missing");
        Ok(())
    }

    pub(crate) fn active_image_rows(&self) -> anyhow::Result<Vec<MigrationImageRow>> {
        let mut statement = self.conn.prepare(
            "SELECT id, content_path, favorite, pinned,
                    created_at, updated_at, COALESCE(sort_rank, updated_at)
             FROM clipboard_items
             WHERE item_type = 'image' AND deleted_at IS NULL
             ORDER BY id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok(MigrationImageRow {
                    id: row.get(0)?,
                    content_path: row.get::<_, Option<String>>(1)?.map(PathBuf::from),
                    favorite: row.get::<_, i64>(2)? == 1,
                    pinned: row.get::<_, i64>(3)? == 1,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    sort_rank: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, rusqlite::Error>>()?;
        Ok(rows)
    }

    pub(crate) fn commit_image_migration(
        &self,
        name: &str,
        merges: &[ImageMigrationMerge],
    ) -> anyhow::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;
        let now = Utc::now().timestamp_micros();
        for merge in merges {
            for duplicate_id in &merge.duplicate_ids {
                let changed = transaction.execute(
                    "UPDATE clipboard_items SET deleted_at = ?1
                     WHERE id = ?2 AND item_type = 'image' AND deleted_at IS NULL",
                    params![now, duplicate_id],
                )?;
                anyhow::ensure!(changed == 1, "active duplicate image row is missing");
                transaction.execute(
                    "DELETE FROM clipboard_items_fts WHERE id = ?1",
                    params![duplicate_id],
                )?;
            }
        }
        for merge in merges {
            let changed = transaction.execute(
                "UPDATE clipboard_items
                 SET hash = ?1, content_hash = NULL
                 WHERE id = ?2 AND item_type = 'image' AND deleted_at IS NULL",
                params![
                    format!("__image_migration__:{name}:{}", merge.retained_id),
                    merge.retained_id
                ],
            )?;
            anyhow::ensure!(changed == 1, "retained image row is missing");
        }
        for merge in merges {
            let changed = transaction.execute(
                "UPDATE clipboard_items
                 SET hash = ?1, content_hash = ?2, content_path = ?3,
                     favorite = ?4, pinned = ?5, created_at = ?6, updated_at = ?7,
                     sort_rank = ?8, size_bytes = ?9
                 WHERE id = ?10 AND item_type = 'image' AND deleted_at IS NULL",
                params![
                    format!("image:{}", merge.content_hash),
                    merge.content_hash,
                    merge.content_path.to_string_lossy(),
                    i64::from(merge.favorite),
                    i64::from(merge.pinned),
                    merge.created_at,
                    merge.updated_at,
                    merge.sort_rank,
                    merge.size_bytes,
                    merge.retained_id
                ],
            )?;
            anyhow::ensure!(changed == 1, "retained image row is missing");
            enqueue_cleanup_paths(&transaction, &merge.obsolete_paths)?;
        }
        let changed = transaction.execute(
            "UPDATE schema_migrations SET state = 'cleanup_pending', updated_at = ?1
             WHERE name = ?2 AND state = 'pending'",
            params![now, name],
        )?;
        anyhow::ensure!(changed == 1, "pending migration row is missing");
        transaction.commit()?;
        Ok(())
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
                "UPDATE clipboard_items
                 SET updated_at = ?1,
                     sort_rank = ?1,
                     source_app = COALESCE(NULLIF(TRIM(?2), ''), source_app)
                 WHERE id = ?3",
                params![now, draft.source_app.as_deref(), existing_id],
            )?;
            return self
                .get_item(&existing_id)?
                .ok_or_else(|| anyhow::anyhow!("item missing"));
        }

        let id = Uuid::new_v4().to_string();
        let item_type = format!("{:?}", draft.item_type).to_lowercase();
        self.conn.execute(
            "INSERT INTO clipboard_items
            (id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, sort_rank, created_at, updated_at, content_hash)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?9, ?10, ?11)",
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
                now,
                draft.content_hash
            ],
        )?;
        self.rebuild_fts_for_item(&id)?;
        self.get_item(&id)?
            .ok_or_else(|| anyhow::anyhow!("inserted item missing"))
    }

    pub fn insert_or_touch_image(
        &self,
        draft: ClipboardItemDraft,
    ) -> anyhow::Result<ClipboardItem> {
        if draft.item_type != ClipboardItemType::Image {
            return Err(anyhow::anyhow!("image draft must use image item type"));
        }
        if draft
            .content_hash
            .as_deref()
            .is_none_or(|content_hash| content_hash.is_empty())
        {
            return Err(anyhow::anyhow!("image draft requires content_hash"));
        }

        let transaction = self.conn.unchecked_transaction()?;
        let item = Self::insert_or_touch_image_in_connection(&transaction, draft)?;
        transaction.commit()?;
        Ok(item)
    }

    fn insert_or_touch_image_in_connection(
        conn: &Connection,
        draft: ClipboardItemDraft,
    ) -> anyhow::Result<ClipboardItem> {
        let hash = draft.stable_hash();
        let content_hash = draft
            .content_hash
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("image draft requires content_hash"))?;
        let now = Utc::now().timestamp_micros();

        let id = if let Some(existing_id) = conn
            .query_row(
                "SELECT id FROM clipboard_items
                 WHERE item_type = 'image' AND content_hash = ?1 AND deleted_at IS NULL",
                params![content_hash],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            conn.execute(
                "UPDATE clipboard_items
                 SET updated_at = ?1,
                     sort_rank = ?1,
                     source_app = COALESCE(NULLIF(TRIM(?2), ''), source_app)
                 WHERE id = ?3",
                params![now, draft.source_app.as_deref(), existing_id],
            )?;
            existing_id
        } else {
            let id = Uuid::new_v4().to_string();
            let item_type = format!("{:?}", draft.item_type).to_lowercase();
            conn.execute(
                "INSERT INTO clipboard_items
                (id, hash, item_type, content, content_path, preview, source_app, favorite, size_bytes, sort_rank, created_at, updated_at, content_hash)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?9, ?10, ?11)",
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
                    now,
                    draft.content_hash
                ],
            )?;
            id
        };

        Self::rebuild_fts_for_item_on(conn, &id)?;
        conn.query_row(
            "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
             FROM clipboard_items
             WHERE id = ?1 AND deleted_at IS NULL",
            params![id],
            Self::map_item,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("image item missing"))
    }

    pub fn find_active_image(&self, content_hash: &str) -> anyhow::Result<Option<ClipboardItem>> {
        self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
                 FROM clipboard_items
                 WHERE item_type = 'image'
                   AND content_hash = ?1
                   AND deleted_at IS NULL",
                params![content_hash],
                Self::map_item,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn active_blob_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut statement = self.conn.prepare(
            "SELECT DISTINCT content_path
             FROM clipboard_items
             WHERE item_type = 'image'
               AND content_path IS NOT NULL
               AND deleted_at IS NULL
             ORDER BY content_path",
        )?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn update_image_references_and_enqueue_cleanup(
        &self,
        updates: &[ImageReferenceUpdate],
        cleanup_paths: &[PathBuf],
    ) -> anyhow::Result<()> {
        let transaction = self.conn.unchecked_transaction()?;

        enqueue_cleanup_paths(&transaction, cleanup_paths)?;

        for update in updates {
            if update.content_hash.is_empty() {
                return Err(anyhow::anyhow!("image reference requires content_hash"));
            }
            let changed = transaction.execute(
                "UPDATE clipboard_items
                 SET hash = ?1, content_hash = ?2, content_path = ?3
                 WHERE id = ?4 AND item_type = 'image' AND deleted_at IS NULL",
                params![
                    format!("image:{}", update.content_hash),
                    update.content_hash,
                    update.content_path.to_string_lossy(),
                    update.item_id
                ],
            )?;
            if changed != 1 {
                return Err(anyhow::anyhow!(
                    "active image item missing: {}",
                    update.item_id
                ));
            }
        }

        transaction.commit()?;
        Ok(())
    }

    pub fn pending_cleanup_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut statement = self
            .conn
            .prepare("SELECT path FROM blob_cleanup_queue ORDER BY created_at, path")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map(PathBuf::from))
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn complete_cleanup_path(&self, path: &Path) -> anyhow::Result<()> {
        self.conn.execute(
            "DELETE FROM blob_cleanup_queue WHERE path = ?1",
            params![path.to_string_lossy()],
        )?;
        Ok(())
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
        cursor: Option<SearchCursor>,
    ) -> anyhow::Result<ClipboardSearchPage> {
        let mut sql = String::from(
            "SELECT id, hash, item_type, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at,
                    COALESCE(sort_rank, updated_at) AS effective_rank
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
            sql.push_str(
                " AND (
                    pinned < ?
                    OR (pinned = ? AND (
                        COALESCE(sort_rank, updated_at) < ?
                        OR (COALESCE(sort_rank, updated_at) = ? AND (
                            updated_at < ?
                            OR (updated_at = ? AND id > ?)
                        ))
                    ))
                )",
            );
            let pinned = i64::from(cursor.pinned);
            sql_params.push(Value::Integer(pinned));
            sql_params.push(Value::Integer(pinned));
            sql_params.push(Value::Integer(cursor.effective_rank));
            sql_params.push(Value::Integer(cursor.effective_rank));
            sql_params.push(Value::Integer(cursor.updated_at));
            sql_params.push(Value::Integer(cursor.updated_at));
            sql_params.push(Value::Text(cursor.id));
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
            " ORDER BY pinned DESC, COALESCE(sort_rank, updated_at) DESC, updated_at DESC, id ASC LIMIT ?",
        );
        sql_params.push(Value::Integer(limit.saturating_add(1)));

        let mut statement = self.conn.prepare(&sql)?;
        let rows = statement.query_map(params_from_iter(sql_params), |row| {
            Ok((Self::map_summary(row)?, row.get::<_, i64>(11)?))
        })?;
        let mut rows = rows.collect::<Result<Vec<_>, _>>()?;
        let has_more = rows.len() > usize::try_from(limit).unwrap_or(usize::MAX);
        if has_more {
            rows.pop();
        }
        let next_cursor = if has_more {
            rows.last().map(|(item, effective_rank)| SearchCursor {
                pinned: item.pinned,
                effective_rank: *effective_rank,
                updated_at: item.updated_at,
                id: item.id.clone(),
            })
        } else {
            None
        };

        Ok(ClipboardSearchPage {
            items: rows.into_iter().map(|(item, _)| item).collect(),
            next_cursor,
        })
    }

    pub fn list_items_for_backup(&self, limit: i64) -> anyhow::Result<Vec<ClipboardItem>> {
        let mut statement = self.conn.prepare(
            "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
             FROM clipboard_items
             WHERE deleted_at IS NULL
             ORDER BY pinned DESC, COALESCE(sort_rank, updated_at) DESC, updated_at DESC, id ASC
             LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], Self::map_item)?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_item(&self, id: &str) -> anyhow::Result<Option<ClipboardItem>> {
        self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
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
        let transaction = self.conn.unchecked_transaction()?;
        enqueue_cleanup_paths(&transaction, &blob_paths)?;
        transaction.execute(
            "UPDATE clipboard_items SET deleted_at = ?1 WHERE id = ?2",
            params![Utc::now().timestamp_micros(), id],
        )?;
        transaction.execute("DELETE FROM clipboard_items_fts WHERE id = ?1", params![id])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn prune_history(&self, max_history_items: i64, retention_days: i64) -> anyhow::Result<()> {
        let now = Utc::now().timestamp_micros();

        if retention_days > 0 {
            let cutoff = now - retention_days.saturating_mul(24 * 60 * 60 * 1_000_000);
            let blob_paths = self.content_paths_for_retention_cutoff(cutoff)?;
            let transaction = self.conn.unchecked_transaction()?;
            enqueue_cleanup_paths(&transaction, &blob_paths)?;
            transaction.execute(
                "UPDATE clipboard_items
                 SET deleted_at = ?1
                 WHERE deleted_at IS NULL
                   AND favorite = 0
                   AND updated_at < ?2",
                params![now, cutoff],
            )?;
            transaction.commit()?;
        }

        if max_history_items > 0 {
            let blob_paths = self.content_paths_over_limit(max_history_items)?;
            let transaction = self.conn.unchecked_transaction()?;
            enqueue_cleanup_paths(&transaction, &blob_paths)?;
            transaction.execute(
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
            transaction.commit()?;
        }

        self.remove_deleted_from_fts()?;
        Ok(())
    }

    pub(crate) fn capacity_items(&self) -> anyhow::Result<Vec<ClipboardItem>> {
        let mut statement = self.conn.prepare(
            "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
             FROM clipboard_items
             WHERE deleted_at IS NULL",
        )?;
        let rows = statement.query_map([], Self::map_item)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn soft_delete_batch(&self, ids: &[String]) -> anyhow::Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let blob_paths = self.content_paths_for_ids(ids)?;
        let transaction = self.conn.unchecked_transaction()?;
        enqueue_cleanup_paths(&transaction, &blob_paths)?;
        let now = Utc::now().timestamp_micros();
        for id in ids {
            let changed = transaction.execute(
                "UPDATE clipboard_items
                 SET deleted_at = ?1
                 WHERE id = ?2 AND deleted_at IS NULL AND favorite = 0",
                params![now, id],
            )?;
            anyhow::ensure!(changed == 1, "capacity prune candidate changed: {id}");
            transaction.execute("DELETE FROM clipboard_items_fts WHERE id = ?1", params![id])?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn clear_history(&self) -> anyhow::Result<()> {
        let blob_paths = self.active_blob_paths()?;
        let transaction = self.conn.unchecked_transaction()?;
        enqueue_cleanup_paths(&transaction, &blob_paths)?;
        transaction.execute("DELETE FROM clipboard_items_fts", [])?;
        transaction.execute("DELETE FROM clipboard_items", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn find_by_hash(&self, hash: &str) -> anyhow::Result<Option<ClipboardItem>> {
        let item = self.conn
            .query_row(
                "SELECT id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash
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
                        content_hash: row.get(12)?,
                    })
                },
            )
            .optional()?;
        Ok(item)
    }

    pub fn insert_imported_item(&self, item: &ClipboardItem) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO clipboard_items (id, hash, item_type, content, content_path, preview, source_app, favorite, pinned, size_bytes, created_at, updated_at, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
                item.content_hash,
            ],
        )?;

        // 更新 FTS 索引
        self.rebuild_fts_for_item(&item.id)?;

        Ok(())
    }

    pub fn import_items_transactionally(
        &self,
        items: &[ClipboardItem],
        mode: RepositoryImportMode,
    ) -> anyhow::Result<RepositoryImportResult> {
        let transaction = self.conn.unchecked_transaction()?;
        if mode == RepositoryImportMode::Overwrite {
            let final_paths = items
                .iter()
                .filter(|item| item.item_type == "image")
                .filter_map(|item| item.content_path.as_deref())
                .map(PathBuf::from)
                .collect::<HashSet<_>>();
            let old_paths = active_blob_paths_on(&transaction)?
                .into_iter()
                .filter(|path| !final_paths.contains(path))
                .collect::<Vec<_>>();
            enqueue_cleanup_paths(&transaction, &old_paths)?;
            transaction.execute("DELETE FROM clipboard_items_fts", [])?;
            transaction.execute("DELETE FROM clipboard_items", [])?;
        }

        let mut imported = 0usize;
        let mut skipped = 0usize;
        for item in items {
            if mode == RepositoryImportMode::Merge {
                let hash_exists = transaction
                    .query_row(
                        "SELECT 1 FROM clipboard_items WHERE hash = ?1 AND deleted_at IS NULL",
                        params![item.hash],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if hash_exists {
                    skipped = skipped
                        .checked_add(1)
                        .ok_or_else(|| anyhow::anyhow!("import skipped count overflow"))?;
                    continue;
                }
                let id_exists = transaction
                    .query_row(
                        "SELECT 1 FROM clipboard_items WHERE id = ?1",
                        params![item.id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                anyhow::ensure!(
                    !id_exists,
                    "import item id conflicts with existing row: {}",
                    item.id
                );
            }
            insert_imported_item_on(&transaction, item)?;
            Self::rebuild_fts_for_item_on(&transaction, &item.id)?;
            imported = imported
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("imported item count overflow"))?;
        }
        transaction.commit()?;
        Ok(RepositoryImportResult { imported, skipped })
    }

    pub fn any_active_blob_path(&self, paths: &[PathBuf]) -> anyhow::Result<bool> {
        for path in paths {
            if self
                .conn
                .query_row(
                    "SELECT 1 FROM clipboard_items
                     WHERE item_type = 'image' AND content_path = ?1 AND deleted_at IS NULL
                     LIMIT 1",
                    params![path.to_string_lossy()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn active_hashes(&self) -> anyhow::Result<HashSet<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT hash FROM clipboard_items WHERE deleted_at IS NULL")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<HashSet<_>, _>>().map_err(Into::into)
    }

    fn rebuild_fts_for_item(&self, id: &str) -> anyhow::Result<()> {
        Self::rebuild_fts_for_item_on(&self.conn, id)
    }

    fn rebuild_fts_for_item_on(conn: &Connection, id: &str) -> anyhow::Result<()> {
        conn.execute("DELETE FROM clipboard_items_fts WHERE id = ?1", params![id])?;
        conn.execute(
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
                    "SELECT content_path FROM clipboard_items
                     WHERE id = ?1 AND item_type = 'image' AND deleted_at IS NULL",
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
               AND item_type = 'image'
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
               AND item_type = 'image'
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
            content_hash: row.get(12)?,
        })
    }

    fn map_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardItemSummary> {
        let item_type: String = row.get(2)?;
        let content_path: Option<String> = row.get(3)?;
        let thumbnail_path = if item_type == "image" {
            content_path.as_deref().map(|path| {
                crate::blobs::thumbnail_path_for(Path::new(path))
                    .to_string_lossy()
                    .into_owned()
            })
        } else {
            None
        };

        Ok(ClipboardItemSummary {
            id: row.get(0)?,
            hash: row.get(1)?,
            item_type,
            preview: row.get(4)?,
            source_app: row.get(5)?,
            favorite: row.get::<_, i64>(6)? == 1,
            pinned: row.get::<_, i64>(7)? == 1,
            size_bytes: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
            content_path,
            thumbnail_path,
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

    if !columns.iter().any(|name| name == "content_hash") {
        conn.execute(
            "ALTER TABLE clipboard_items ADD COLUMN content_hash TEXT",
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

fn enqueue_cleanup_paths(conn: &Connection, paths: &[PathBuf]) -> anyhow::Result<()> {
    let now = Utc::now().timestamp_micros();
    for path in paths {
        let thumbnail_path = crate::blobs::thumbnail_path_for(path);
        for cleanup_path in [path.as_path(), thumbnail_path.as_path()] {
            conn.execute(
                "INSERT OR IGNORE INTO blob_cleanup_queue(path, created_at) VALUES (?1, ?2)",
                params![cleanup_path.to_string_lossy(), now],
            )?;
        }
    }
    Ok(())
}

fn active_blob_paths_on(conn: &Connection) -> anyhow::Result<Vec<PathBuf>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT content_path
         FROM clipboard_items
         WHERE item_type = 'image' AND content_path IS NOT NULL AND deleted_at IS NULL",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| row.map(PathBuf::from))
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn insert_imported_item_on(conn: &Connection, item: &ClipboardItem) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO clipboard_items
         (id, hash, item_type, content, content_path, content_hash, preview, source_app,
          favorite, pinned, size_bytes, sort_rank, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            item.id,
            item.hash,
            item.item_type,
            item.content,
            item.content_path,
            item.content_hash,
            item.preview,
            item.source_app,
            item.favorite,
            item.pinned,
            item.size_bytes,
            item.updated_at,
            item.created_at,
            item.updated_at,
        ],
    )?;
    Ok(())
}
