# History Pagination And Source Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development and execute each task in order. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Incrementally load all history records and persist the Windows source process for new captures.

**Architecture:** Return paged search results with an opaque composite cursor aligned to repository ordering. Keep frontend pagination state generation-safe and resolve source process metadata once per clipboard read before constructing typed captures.

**Tech Stack:** React 18, TypeScript, Node test runner, Rust, rusqlite, windows-sys, Tauri 2.

---

### Task 1: Composite backend pagination

**Files:**
- Modify: `src-tauri/src/storage/repository.rs`
- Modify: `src-tauri/src/commands.rs`

- [ ] Add failing repository tests for pinned, reordered, and tied rows across pages.
- [ ] Run the targeted Rust tests and verify the pagination regression fails.
- [ ] Add serializable search page/cursor types and lexicographic cursor predicates.
- [ ] Return an opaque next cursor from `search_items`.
- [ ] Run the targeted tests and verify they pass.

### Task 2: Frontend automatic loading

**Files:**
- Modify: `src/lib/history-ui.js`
- Modify: `src/lib/history-ui.d.ts`
- Modify: `src/lib/history-ui.test.js`
- Modify: `src/features/history/api.ts`
- Modify: `src/App.tsx`

- [ ] Add failing unit tests for bottom-load eligibility and page deduplication.
- [ ] Run the targeted Node tests and verify the new tests fail.
- [ ] Implement the pagination helpers and typed page API.
- [ ] Add reset, append, exhaustion, stale-response, and in-flight state handling.
- [ ] Trigger the next page near the scroll bottom.
- [ ] Run frontend tests and type checking.

### Task 3: Windows source process capture

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/src/clipboard/types.rs`
- Modify: `src-tauri/src/clipboard/win.rs`
- Modify: `src-tauri/src/clipboard/mod.rs`
- Modify: `src-tauri/src/storage/repository.rs`

- [ ] Add failing tests for image source propagation and duplicate-source refresh.
- [ ] Run targeted Rust tests and verify the regressions fail.
- [ ] Resolve the clipboard owner executable name with Windows APIs.
- [ ] Carry the source through text, files, and image captures.
- [ ] Update duplicate rows only when a new non-empty source is available.
- [ ] Run targeted and full Rust tests.

### Task 4: Full verification

**Files:**
- Verify all modified files.

- [ ] Run `npm test`.
- [ ] Run `npm run typecheck`.
- [ ] Run `cargo test --manifest-path src-tauri/Cargo.toml`.
- [ ] Run `npm run tauri build` or the repository release-equivalent build.
- [ ] Inspect `git diff --check` and the final scoped diff.
