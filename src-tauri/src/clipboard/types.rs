use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClipboardItemType {
    Text,
    Html,
    Image,
    Files,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardItemDraft {
    pub item_type: ClipboardItemType,
    pub content: Option<String>,
    pub content_path: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    pub preview: String,
    pub source_app: Option<String>,
    pub size_bytes: i64,
}

impl ClipboardItemDraft {
    pub fn stable_hash(&self) -> String {
        if self.item_type == ClipboardItemType::Image {
            if let Some(content_hash) = self.content_hash.as_deref() {
                return format!("image:{content_hash}");
            }
        }

        let mut hasher = Sha256::new();
        hasher.update(format!("{:?}", self.item_type));
        hasher.update(self.content.as_deref().unwrap_or_default());
        hasher.update(self.content_path.as_deref().unwrap_or_default());
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_hash_is_same_for_same_content() {
        let draft = ClipboardItemDraft {
            item_type: ClipboardItemType::Text,
            content: Some("hello".to_string()),
            content_path: None,
            content_hash: None,
            preview: "hello".to_string(),
            source_app: None,
            size_bytes: 5,
        };

        assert_eq!(draft.stable_hash(), draft.stable_hash());
    }

    #[test]
    fn stable_hash_differs_by_type() {
        let text = ClipboardItemDraft {
            item_type: ClipboardItemType::Text,
            content: Some("hello".to_string()),
            content_path: None,
            content_hash: None,
            preview: "hello".to_string(),
            source_app: None,
            size_bytes: 5,
        };
        let html = ClipboardItemDraft {
            item_type: ClipboardItemType::Html,
            ..text.clone()
        };

        assert_ne!(text.stable_hash(), html.stable_hash());
    }

    #[test]
    fn image_capture_keeps_owned_dib_deferred() {
        let capture = ClipboardCapture::ImageDib(vec![1, 2, 3, 4]);

        match capture {
            ClipboardCapture::ImageDib(dib) => assert_eq!(dib, vec![1, 2, 3, 4]),
            ClipboardCapture::Draft(_) => panic!("image must not be persisted by the adapter"),
        }
    }
}

#[derive(Debug)]
pub enum ClipboardCapture {
    Draft(ClipboardItemDraft),
    ImageDib(Vec<u8>),
}
