# Clipboard Image Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop self-generated clipboard recapture and store one canonical blob for each unique image while safely migrating existing data.

**Architecture:** Introduce a semantic image identity and an `ImageBlobStore` read/write coordinator. Clipboard events carry Windows sequence numbers; capture stages canonical files before short SQLite transactions; startup migration uses a durable state and cleanup queue.

**Tech Stack:** Rust 2021, windows-sys, image, sha2, rusqlite, Tauri 2

---

## File structure

- Create `src-tauri/src/blobs/image.rs`: DIB decoding, semantic hashing, canonical names, staging.
- Create `src-tauri/src/blobs/store.rs`: `ImageBlobStore` read/write leases, managed usage/free-space queries, and file install/rollback.
- Create `src-tauri/src/clipboard/sequence.rs`: pure clipboard sequence state.
- Create `src-tauri/src/storage/image_migration.rs`: backed-up resumable migration.
- Modify `src-tauri/src/clipboard/{mod.rs,types.rs,win.rs}`, `storage/{schema.rs,repository.rs,mod.rs}`, `blobs/mod.rs`, `main.rs`, and `commands.rs`.

### Task 1: Semantic image identity and blob coordinator

**Files:**
- Create: `src-tauri/src/blobs/image.rs`
- Create: `src-tauri/src/blobs/store.rs`
- Modify: `src-tauri/src/blobs/mod.rs`

- [ ] **Step 1: Write failing tests** for a 1x1 DIB, the same DIB with trailing `GlobalSize` padding, equivalent BITMAPINFOHEADER/BITMAPV5HEADER pixels, different pixels, invalid DIB rejection, canonical names, read/write lease exclusion, and install rollback.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml blobs:: -- --nocapture`.
- [ ] **Step 3: Implement the public contracts:**

```rust
pub struct ImageIdentity { pub content_hash: String, pub width: u32, pub height: u32 }
pub fn image_identity_from_dib(dib: &[u8]) -> anyhow::Result<ImageIdentity>;

pub struct StagedImage {
    pub content_hash: String,
    pub bmp_path: PathBuf,
    pub thumbnail_path: PathBuf,
    pub bmp_size: u64,
    pub thumbnail_size: u64,
}
pub fn stage_dib(stage_root: &Path, dib: Vec<u8>) -> anyhow::Result<StagedImage>;

pub struct ImageBlobStore { /* blob_dir, stage_dir, RwLock */ }
impl ImageBlobStore {
    pub fn with_read<T>(&self, f: impl FnOnce(&Path) -> anyhow::Result<T>) -> anyhow::Result<T>;
    pub fn with_write<T>(&self, f: impl FnOnce(&Path, &Path) -> anyhow::Result<T>) -> anyhow::Result<T>;
}
```

Hash exactly `SCIMG1 || width_le || height_le || rgba8`; never hash allocation padding. `stage_dib` owns the DIB buffer, writes and flushes the BMP, explicitly drops the DIB, and only then reopens the staged BMP to decode/generate the thumbnail. Flush both staged files. Roll back only paths created by the current install. Add `managed_usage()` and platform free-space queries here so migration and later capacity policy share one implementation.
- [ ] **Step 4: Verify GREEN** with the focused command above.
- [ ] **Step 5: Commit** as `feat: add semantic image blob store`.

### Task 2: Repository identity and durable cleanup state

**Files:**
- Modify: `src-tauri/src/storage/schema.rs`
- Modify: `src-tauri/src/storage/repository.rs`

- [ ] **Step 1: Write failing tests** that migrate a legacy DB, add nullable `content_hash`, enforce one active image per content hash, allow a soft-deleted duplicate, create `schema_migrations` and `blob_cleanup_queue`, and roll back reference changes and cleanup rows together.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml storage::repository::tests -- --nocapture`.
- [ ] **Step 3: Implement repository-only contracts** for `find_active_image`, `insert_or_touch_image`, `active_blob_paths`, atomic reference-update-plus-cleanup enqueue, listing cleanup rows, and completing one cleanup row. Images use `hash = "image:" + content_hash`; repository methods never delete files.
- [ ] **Step 4: Verify GREEN** with the focused command.
- [ ] **Step 5: Commit** as `feat: add image identity schema and cleanup queue`.

### Task 3: Exact clipboard sequence suppression

**Files:**
- Create: `src-tauri/src/clipboard/sequence.rs`
- Modify: `src-tauri/src/clipboard/mod.rs`
- Modify: `src-tauri/src/clipboard/win.rs`

- [ ] **Step 1: Write failing pure tests** for an exact internal match, duplicate notification, two queued internal writes, later external sequence, stale worker event, bounded eviction, and sequence `0` fail-open.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml clipboard::sequence::tests -- --nocapture`.
- [ ] **Step 3: Implement:**

```rust
pub enum SequenceDecision { SuppressInternal, IgnoreDuplicate, Capture }
pub struct ClipboardSequenceState { /* recent internal sequences + last enqueued */ }
impl ClipboardSequenceState {
    pub fn register_internal(&mut self, sequence: u32);
    pub fn classify_notification(&mut self, sequence: u32) -> SequenceDecision;
}
```

Each successful text/HTML/file/image writer records `GetClipboardSequenceNumber` while `ClipboardGuard` is still open. `window_proc` sends a `u32` only for `Capture`; the worker discards it if the current sequence has advanced. Try `CF_DIBV5` before `CF_DIB`.
- [ ] **Step 4: Verify GREEN** with the focused test and `cargo check --manifest-path src-tauri/Cargo.toml`. Do not run ignored real-clipboard tests without authorization.
- [ ] **Step 5: Commit** as `fix: suppress internally generated clipboard events`.

### Task 4: Content-addressed capture and leased reads

**Files:**
- Modify: `src-tauri/src/clipboard/{mod.rs,types.rs,win.rs}`
- Modify: `src-tauri/src/blobs/{image.rs,store.rs}`
- Modify: `src-tauri/src/storage/repository.rs`
- Modify: `src-tauri/src/{commands.rs,main.rs}`

- [ ] **Step 1: Write failing integration tests** proving first capture creates one BMP/thumbnail/row, equivalent pixels only touch the row, the owned DIB is released before thumbnail decode, repository failure removes only newly created files, paste holds a blob read lease after releasing the repository lock, and delete/clear/prune cannot remove a blob during a read lease or while another active row still references it.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml duplicate_image -- --nocapture`.
- [ ] **Step 3: Implement deferred persistence:** the Windows adapter transfers an owned DIB `Vec<u8>` without UUID file creation; the listener computes identity and checks the repository, transfers ownership to `stage_dib`, which drops the DIB after the BMP flush and before on-disk thumbnail decode, then holds the blob write lease while installing and calling `insert_or_touch_image`. On a unique-index race, touch the winner and roll back loser-created paths.
- [ ] **Step 4: Update all blob access paths.** `copy_item` fetches metadata, releases the repository lock, then holds a blob read lease from before file open through clipboard write. `delete_item`, `clear_history`, and prune hold the blob write lease, commit DB deletion first, query active references, then remove only unreferenced BMP/thumbnail paths. Keep path-containment validation. Store `Arc<ImageBlobStore>` in `AppState`.
- [ ] **Step 5: Verify GREEN** with the focused test plus `cargo test --manifest-path src-tauri/Cargo.toml commands::tests -- --nocapture`.
- [ ] **Step 6: Commit** as `feat: store clipboard images by content hash`.

### Task 5: Backed-up resumable migration

**Files:**
- Create: `src-tauri/src/storage/image_migration.rs`
- Modify: `src-tauri/src/storage/{mod.rs,repository.rs}`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Write failing tests** for mandatory DB/WAL/SHM backup; persistent backup-path reuse across repeated `pending` failures; free-space preflight before staging; earliest-row retention; OR favorite/pinned; earliest `created_at`; latest `updated_at` and `sort_rank`; canonical preparation before reference commit; FTS cleanup; durable cleanup queue; recovery from `pending` and `cleanup_pending`; no repeated backup after completion; and post-cleanup managed-usage/quota status before listener startup.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml storage::image_migration::tests -- --nocapture`.
- [ ] **Step 3: Implement the state machine** after repository open and before listener startup. Hold the blob write lease; checkpoint and create exactly one backup whose path is stored in the `pending` state and reused on retry; scan/decode/hash and calculate required temporary bytes; abort before staging when platform free space is insufficient; stage and flush canonical files; commit merged references, soft deletes, FTS changes, cleanup rows, and state together; process cleanup idempotently; mark complete only when the queue is empty. Recompute managed usage after cleanup and pass a quota-blocked startup status when usage still exceeds 5 GiB so image capture cannot resume until later capacity pruning/policy permits it.
- [ ] **Step 4: Verify GREEN** with the focused test and `cargo test --manifest-path src-tauri/Cargo.toml`.
- [ ] **Step 5: Commit** as `feat: migrate duplicate image blobs safely`.