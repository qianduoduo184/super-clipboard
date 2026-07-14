# History Memory and UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep history-list payloads and image decoding small while preserving full detail and paste behavior.

**Architecture:** Split repository list summaries from full item details. Expose thumbnail and original paths separately; the React list uses lazy thumbnails and fetches full detail only for the selected item.

**Tech Stack:** Rust 2021, rusqlite, Tauri 2, React 18, TypeScript

---

**Prerequisite and execution order:** Complete the clipboard image core plan before this plan. Do not execute it in parallel with the core or backup/capacity plans because they modify shared DTO and command files.

## File structure

- Modify `src-tauri/src/storage/repository.rs`: `ClipboardItemSummary` query without full `content`.
- Modify `src-tauri/src/commands.rs`: summary search and full-detail commands.
- Modify `src/features/history/api.ts`, `src/lib/clipboard-adapter.{js,d.ts}`, and `src/App.tsx`: separate summary/detail models.
- Modify `src-tauri/src/blobs/mod.rs`: bounded BMP-to-DIB read without a second payload clone.

### Task 1: Lightweight repository and command DTOs

**Files:**
- Modify: `src-tauri/src/storage/repository.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing Rust tests** asserting search results do not select/serialize `content`, image summaries contain both original and thumbnail paths, and `get_item` still returns full text/HTML.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml lightweight_summary -- --nocapture`.
- [ ] **Step 3: Implement `ClipboardItemSummary`** with id, hash, type, preview, source, favorite, pinned, size, timestamps, `content_path`, and `thumbnail_path`. Keep `ClipboardItem` as the detail/export model. Update search command return type and leave `get_item` full fidelity.
- [ ] **Step 4: Verify GREEN** with the focused test and `cargo check --manifest-path src-tauri/Cargo.toml`.
- [ ] **Step 5: Commit** as `perf: return lightweight history summaries`.

### Task 2: On-demand details and lazy thumbnails

**Files:**
- Modify: `src/features/history/api.ts`
- Modify: `src/lib/clipboard-adapter.js`
- Modify: `src/lib/clipboard-adapter.d.ts`
- Modify: `src/lib/clipboard-adapter.test.js`
- Modify: `src/App.tsx`

- [ ] **Step 1: Write failing frontend tests** for `thumbnail_path -> thumbnailPath`, preserved `contentPath`, and detail merge by id without replacing list ordering.
- [ ] **Step 2: Verify RED** with `npm test -- src/lib/clipboard-adapter.test.js`.
- [ ] **Step 3: Implement summary/detail types and mapping.** The list image must render `thumbnailPath` with `loading="lazy"` and `decoding="async"`; the detail pane uses `contentPath`. Selection triggers `getItemDetail(id)`, ignores stale responses after selection changes, and text/HTML detail renders fetched full content rather than the truncated preview.
- [ ] **Step 4: Verify GREEN** with `npm test` and `npm run typecheck`.
- [ ] **Step 5: Commit** as `perf: load thumbnails and details on demand`.

### Task 3: Bound image paste allocations

**Files:**
- Modify: `src-tauri/src/blobs/mod.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] **Step 1: Write failing tests** for BMP header validation, returning only the DIB payload, rejecting files over 100 MiB, and ensuring copy logic releases repository access before file I/O.
- [ ] **Step 2: Verify RED** with `cargo test --manifest-path src-tauri/Cargo.toml dib_from_bmp -- --nocapture`.
- [ ] **Step 3: Implement a bounded reader** that validates the 14-byte BMP header and reads the remaining payload once. Pass the borrowed/single-owned buffer directly to the Windows allocation boundary; do not create a second `Vec` solely to strip the header. Drop large buffers immediately after use.
- [ ] **Step 4: Verify GREEN** with the focused test, `cargo test --manifest-path src-tauri/Cargo.toml`, and `npm run build:frontend`.
- [ ] **Step 5: Commit** as `perf: reduce clipboard image memory copies`.
