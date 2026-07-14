pub const INIT_SQL: &str = r#"
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;

CREATE TABLE IF NOT EXISTS clipboard_items (
  id TEXT PRIMARY KEY,
  hash TEXT NOT NULL,
  item_type TEXT NOT NULL,
  content TEXT,
  content_path TEXT,
  content_hash TEXT,
  preview TEXT NOT NULL,
  source_app TEXT,
  favorite INTEGER NOT NULL DEFAULT 0,
  pinned INTEGER NOT NULL DEFAULT 0,
  size_bytes INTEGER NOT NULL DEFAULT 0,
  sort_rank INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER
);

CREATE VIRTUAL TABLE IF NOT EXISTS clipboard_items_fts
USING fts5(id UNINDEXED, preview, content, tokenize='trigram');

CREATE TABLE IF NOT EXISTS schema_migrations (
  name TEXT PRIMARY KEY,
  state TEXT NOT NULL,
  backup_path TEXT,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS blob_cleanup_queue (
  path TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL
);
"#;

pub const INDEX_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS idx_clipboard_items_updated_at
ON clipboard_items(updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_clipboard_items_active_hash
ON clipboard_items(hash)
WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_clipboard_items_active_image_content_hash
ON clipboard_items(content_hash)
WHERE item_type = 'image'
  AND content_hash IS NOT NULL
  AND deleted_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_clipboard_items_type_updated_at
ON clipboard_items(item_type, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_clipboard_items_pinned_updated_at
ON clipboard_items(pinned DESC, updated_at DESC);
"#;
