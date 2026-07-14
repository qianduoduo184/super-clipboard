# Backup Capacity and Logging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stream backups with transactional restore, keep managed blob storage within 5 GiB, limit pruning work, and rotate diagnostics.

**Architecture:** Add a versioned ZIP service that stages and validates imports before one repository transaction under the blob write lease. Centralize physical usage and byte-pressure decisions in the blob store; run pruning at startup and at most every ten minutes; rotate logs under the existing log lock.

**Tech Stack:** Rust 2021, zip, serde_json, rusqlite, Tauri 2

---

**Prerequisite and execution order:** Complete the clipboard image core plan first, then the history memory/UI plan, then execute this plan. Do not run these plans in parallel because they intentionally modify shared storage and command files.

## File structure

- Create `src-tauri/src/backup.rs`: ZIP manifest, streaming export, staged JSON/ZIP import.
- Create `src-tauri/src/storage/capacity.rs`: usage projection, reservation, and prune throttle.
- Modify `src-tauri/src/storage/repository.rs`: transactional merge/overwrite and byte-pressure candidates.
- Modify `src-tauri/src/blobs/store.rs`, `commands.rs`, `main.rs`, and `diagnostics.rs`.
- Modify `src/features/history/api.ts` and `src/features/settings/SettingsView.tsx`: ZIP file filters/messages.
- Modify `src-tauri/Cargo.toml`: direct `zip` dependency using a version already resolved by the lockfile.

### Task 1: Managed usage, quota reservation, and prune throttling

**Files:**
- Create: `src-tauri/src/storage/capacity.rs`
- Modify: `src-tauri/src/storage/{mod.rs,repository.rs}`
- Modify: `src-tauri/src/blobs/store.rs`
- Modify: `src-tauri/src/clipboard/mod.rs`
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Write failing tests** for recursive regular-file accounting under `blob_dir`; exact BMP plus 1 MiB thumbnail reservation; duplicate touch bypass; soft-deleted/orphan/temporary inclusion; favorite protection; pinned non-favorite eligibility; age/count/oldest byte-pressure order; insufficient favorite-only capacity; startup prune; and ten-minute throttling.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml storage::capacity::tests -- --nocapture`.
- [ ] **Step 3: Implement constants and contracts:**

```rust
pub const MAX_IMAGE_ALLOCATION: u64 = 100 * 1024 * 1024;
pub const MANAGED_BLOB_QUOTA: u64 = 5 * 1024 * 1024 * 1024;
pub const THUMBNAIL_RESERVATION: u64 = 1024 * 1024;
pub const PRUNE_INTERVAL: Duration = Duration::from_secs(600);

pub fn managed_usage(blob_dir: &Path) -> anyhow::Result<u64>;
pub fn ensure_capture_capacity(/* usage, exact BMP size, candidates */) -> anyhow::Result<()>;
```

Prune files only after DB soft-delete and only while holding the blob write lease. Recompute exact usage after staging and before canonical install; rejection emits a clear status event and leaves history unchanged.
- [ ] **Step 4: Verify GREEN** with the focused test and `cargo test --manifest-path src-tauri/Cargo.toml`.
- [ ] **Step 5: Commit** as `feat: enforce clipboard blob capacity`.

### Task 2: Streaming ZIP export

**Files:**
- Create: `src-tauri/src/backup.rs`
- Modify: `src-tauri/src/commands.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src/features/history/api.ts`
- Modify: `src/features/settings/SettingsView.tsx`

- [ ] **Step 1: Write failing tests** for manifest version 2, each content hash written once, archive-relative paths, a large blob copied through a bounded buffer, export holding a blob read lease while the repository lock is free, and `parse_backup_info` reading metadata from ZIP `manifest.json` as well as legacy JSON.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::export -- --nocapture`.
- [ ] **Step 3: Implement `export_zip_to(path, repository, blob_store)`.** Snapshot metadata under a short repository lock, release it, acquire the blob read lease, write `manifest.json` then unique `blobs/<content_hash>.bmp` entries sequentially with `std::io::copy`. Do not build Base64 or the full archive in memory. Update `parse_backup_info` to detect ZIP, open and validate `manifest.json`, and return the same `BackupInfo` contract; retain legacy JSON parsing.
- [ ] **Step 4: Change dialogs/API** to default to `.zip` while selection, metadata preview, and import accept both `.zip` and legacy `.json`.
- [ ] **Step 5: Verify GREEN** with the focused test, `npm test`, and `npm run typecheck`.
- [ ] **Step 6: Commit** as `feat: stream clipboard backups to zip`.

### Task 3: Transactional ZIP and legacy import

**Files:**
- Modify: `src-tauri/src/backup.rs`
- Modify: `src-tauri/src/storage/repository.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing tests** for ZIP traversal rejection; manifest/blob/hash validation; merge duplicate skip; overwrite replacement; full safety-backup gating; projected post-cleanup quota rejection; transaction rollback; reference changes plus cleanup rows in one transaction; cleanup retry; and legacy JSON passing through the same normalized staging pipeline.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml backup::tests::import -- --nocapture`.
- [ ] **Step 3: Implement staged import.** Extract only validated relative entries to a sibling stage. Process cleanup-pending rows first. Calculate exact projected managed usage from the resulting reference set and staged sizes; reject over 5 GiB. For overwrite, create a complete streaming safety ZIP first. Under the blob write lease install canonical files, then run one merge/overwrite transaction that also enqueues every newly unreferenced old path. On failure preserve old DB/files and remove only new unreferenced installs.
- [ ] **Step 4: Verify GREEN** with the focused tests and all Rust tests.
- [ ] **Step 5: Commit** as `feat: import clipboard backups transactionally`.

### Task 4: Bounded diagnostic logs

**Files:**
- Modify: `src-tauri/src/diagnostics.rs`

- [ ] **Step 1: Write failing tests** using a temporary log path for below-threshold append, 10 MiB rotation, ordered `.log.1` through `.log.3`, oldest deletion, and write failure fallback that does not panic.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml diagnostics::tests -- --nocapture`.
- [ ] **Step 3: Implement rotation** under `LOG_LOCK` before append with `MAX_LOG_BYTES = 10 * 1024 * 1024` and `LOG_GENERATIONS = 3`. Rotation errors fall back to stderr and never fail clipboard operations.
- [ ] **Step 4: Verify GREEN** with focused and all Rust tests.
- [ ] **Step 5: Commit** as `feat: rotate diagnostic logs`.

### Task 5: Integrated verification

**Files:**
- Modify only files required by failures discovered here; do not add unrelated refactors.

- [ ] **Step 1: Run frontend checks:** `npm test`, `npm run typecheck`, `npm run build:frontend`.
- [ ] **Step 2: Run backend checks:** `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo build --manifest-path src-tauri/Cargo.toml --release`.
- [ ] **Step 3: Confirm ignored real-clipboard tests remain ignored** unless the user explicitly authorizes clipboard overwrite.
- [ ] **Step 4: Run `yarn prettier` and `yarn linc` only if available.** In this repository they are currently unavailable; record that fact and use `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` plus the actual npm checks instead.
- [ ] **Step 5: Inspect `git diff --check`, `git status --short`, and commits, then request final code review.**
