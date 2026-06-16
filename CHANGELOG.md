# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
### Changed
### Deprecated
### Removed
### Fixed
### Security

## [0.1.1] - 2026-06-16

### Added
- File list copy/paste support for Windows clipboard (CF_HDROP format)
- Dynamic height virtual scrolling for variable-height list items
- Image rows now display at 3x height (198px) with 120x120px thumbnails
- Friendly error messages for update check failures

### Changed
- Update check now silently fails on startup instead of showing error popups
- Hidden type and source columns in history list for cleaner UI
- Only timestamp is now shown in item metadata
- Manual update check shows user-friendly messages for common scenarios
- Build optimizations: frontend build time reduced by 76% (6.04s → 1.44s)

### Fixed
- Fixed "copy is not implemented for files items" error when copying file lists
- Fixed cargo build error with invalid `jobs=0` setting
- Improved update check UX to avoid technical error messages

### Performance
- Frontend build time: 6.04s → 1.44s (76% improvement)
- Expected Rust build time: 6-8min → 2-3min (50-60% improvement)
- Binary size reduced by ~40-50% through strip and LTO optimizations

## [0.1.0] - 2026-06-10

### Added
- Initial release
- Windows clipboard monitoring with text/HTML/image/file support
- SQLite storage with FTS5 full-text search
- Virtual scrolling for large history lists (tested with 10k items)
- Global shortcut (Ctrl+Shift+V) to open quick panel
- System tray integration
- Auto-start on Windows login
- Favorite and soft-delete support
- Settings page with theme switching
- Auto-update functionality
- Drag-and-drop reordering
- Configurable navigation filters
- ESC key to hide window

### Technical
- Built with Tauri 2.0 + React + TypeScript
- Rust backend with Windows-native clipboard APIs
- WAL mode SQLite for better concurrency
- Image thumbnails with BMP storage
- 33 frontend unit tests with 100% pass rate
- Rust tests covering storage and blob operations

---

## 版本号说明

- **MAJOR**: 不兼容的 API 改动
- **MINOR**: 向后兼容的功能新增
- **PATCH**: 向后兼容的问题修正

## 链接

[Unreleased]: https://github.com/qianduoduo184/super-clipboard/compare/v0.1.1...HEAD
[0.1.1]: https://github.com/qianduoduo184/super-clipboard/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/qianduoduo184/super-clipboard/releases/tag/v0.1.0
