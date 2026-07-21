# History Pagination And Source Capture Design

## Goal

Load the complete clipboard history incrementally as the user scrolls, and record the originating Windows process for newly captured clipboard items.

## History Pagination

The history list loads 50 summaries at a time. Reaching the bottom requests the next page and appends only unseen item IDs. Query, filter, clipboard-change, delete, pin, favorite, and reorder refreshes reset pagination to the first page.

The backend cursor must match the effective SQL order exactly: `pinned DESC`, `COALESCE(sort_rank, updated_at) DESC`, `updated_at DESC`, and `id ASC`. The command returns both items and an opaque next cursor so the frontend does not reconstruct database ordering state.

Only one next-page request may be active at a time. A stale response from an older query/filter generation is ignored. An empty or short page marks the result exhausted.

## Source Capture

At clipboard read time, resolve `GetClipboardOwner()` to its process ID and executable path. Persist only the executable file name (for example `Code.exe`) to avoid exposing document titles or full filesystem paths. Failure to resolve an owner is non-fatal and keeps `source_app` empty.

Text, files, and image captures carry the same resolved source. When duplicate content is copied again, update its `source_app` only when the new capture has a non-empty source, preserving the prior known source when Windows cannot resolve the owner.

Existing rows with a null source cannot be reconstructed and remain displayed as `未知来源`.

## Verification

Repository tests cover composite cursor ordering across pinned and manually reordered rows. Frontend unit tests cover page append, ID deduplication, and bottom-load eligibility. Clipboard and repository tests cover source propagation and duplicate-source refresh. Full frontend tests, type checking, Rust tests, and a production build must pass.
