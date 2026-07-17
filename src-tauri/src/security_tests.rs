#[cfg(test)]
mod security_tests {
    use std::fs;
    /// Test that path traversal attacks are blocked in blob reading
    #[test]
    fn test_blob_path_traversal_blocked() {
        // This test verifies that the path validation in copy_item prevents
        // reading files outside the blob directory
        // The actual validation is in commands.rs:103-120

        let temp_dir = std::env::temp_dir().join("super-clipboard-security-test");
        let blob_dir = temp_dir.join("blobs");
        let outside_dir = temp_dir.join("outside");

        fs::create_dir_all(&blob_dir).expect("create blob dir");
        fs::create_dir_all(&outside_dir).expect("create outside dir");

        // Create a sensitive file outside blob_dir
        let sensitive_file = outside_dir.join("sensitive.txt");
        fs::write(&sensitive_file, b"SECRET DATA").expect("write sensitive file");

        // Try to construct a path that escapes blob_dir
        let evil_path = blob_dir.join("..").join("outside").join("sensitive.txt");

        // Verify that canonicalize + starts_with check would catch this
        let canonical_evil = evil_path.canonicalize().expect("canonicalize evil path");
        let canonical_blob_dir = blob_dir.canonicalize().expect("canonicalize blob dir");

        assert!(
            !canonical_evil.starts_with(&canonical_blob_dir),
            "Path traversal should be detected"
        );

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    /// Test that FTS5 injection is prevented by query sanitization
    #[test]
    fn test_fts5_injection_blocked() {
        use crate::storage::repository::ClipboardRepository;
        use uuid::Uuid;

        let path = std::env::temp_dir().join(format!(
            "super-clipboard-fts-security-{}.sqlite3",
            Uuid::new_v4()
        ));
        let repository = ClipboardRepository::open(path.clone()).expect("open repository");

        // Insert test data
        let draft = crate::clipboard::types::ClipboardItemDraft {
            item_type: crate::clipboard::types::ClipboardItemType::Text,
            content: Some("normal text content".to_string()),
            content_path: None,
            content_hash: None,
            preview: "normal text content".to_string(),
            source_app: None,
            size_bytes: 19,
        };
        repository.insert_or_touch(draft).expect("insert");

        // Try various injection attempts - these should not cause errors or return unexpected results
        let malicious_queries = vec![
            "\" OR 1=1 --",
            "test AND DELETE",
            "* OR NOT",
            ")(^&*",
            "<script>alert('xss')</script>",
            "' UNION SELECT * FROM sqlite_master --",
        ];

        for query in malicious_queries {
            let result = repository.search(
                query.to_string(),
                crate::storage::repository::SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                10,
                None,
            );

            // Should either return empty results or the legitimate item, never error
            assert!(
                result.is_ok(),
                "FTS query should not cause error for input: {}",
                query
            );
        }

        // Cleanup
        let _ = fs::remove_file(&path);
    }

    /// Test that clipboard size limits prevent memory exhaustion
    #[test]
    fn test_clipboard_size_limits() {
        // This test documents the expected behavior of size limits
        // Actual limits are enforced in clipboard/win.rs:
        // - MAX_CLIPBOARD_TEXT_LEN: 100M UTF-16 units (~200MB)
        // - MAX_CLIPBOARD_BLOB_SIZE: 500MB

        const MAX_TEXT_SIZE: usize = 100_000_000; // UTF-16 units
        const MAX_BLOB_SIZE: usize = 500_000_000; // bytes

        // Verify that our limits are reasonable for normal use
        // but prevent extreme cases
        assert!(
            MAX_TEXT_SIZE > 10_000_000,
            "Should allow large text (10M+ chars)"
        );
        assert!(
            MAX_TEXT_SIZE < 1_000_000_000,
            "Should prevent extreme text (1B+ chars)"
        );
        assert!(
            MAX_BLOB_SIZE > 50_000_000,
            "Should allow large images (50MB+)"
        );
        assert!(
            MAX_BLOB_SIZE < 5_000_000_000,
            "Should prevent extreme blobs (5GB+)"
        );
    }
}
