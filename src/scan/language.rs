//! Source-language detection used by extraction and dependency scanning.

use std::path::Path;

/// Languages with a dedicated parser or import resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceLanguage {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Jsx,
    Python,
    Go,
    Unknown,
}

impl SourceLanguage {
    /// Detect a language from a repository-relative path.
    pub fn from_path(path: &str) -> Self {
        let extension = Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase());

        match extension.as_deref() {
            Some("rs") => Self::Rust,
            Some("ts") => Self::TypeScript,
            Some("tsx") => Self::Tsx,
            Some("js" | "mjs" | "cjs") => Self::JavaScript,
            Some("jsx") => Self::Jsx,
            Some("py" | "pyi") => Self::Python,
            Some("go") => Self::Go,
            _ => Self::Unknown,
        }
    }

    pub fn is_tree_sitter_supported(self) -> bool {
        matches!(
            self,
            Self::Rust
                | Self::TypeScript
                | Self::Tsx
                | Self::JavaScript
                | Self::Jsx
                | Self::Python
                | Self::Go
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_supported_extensions() {
        assert_eq!(
            SourceLanguage::from_path("src/lib.rs"),
            SourceLanguage::Rust
        );
        assert_eq!(
            SourceLanguage::from_path("src/component.tsx"),
            SourceLanguage::Tsx
        );
        assert_eq!(
            SourceLanguage::from_path("src/module.mjs"),
            SourceLanguage::JavaScript
        );
        assert_eq!(
            SourceLanguage::from_path("src/types.pyi"),
            SourceLanguage::Python
        );
        assert_eq!(SourceLanguage::from_path("cmd/main.go"), SourceLanguage::Go);
    }

    #[test]
    fn unknown_extensions_use_fallback() {
        assert_eq!(
            SourceLanguage::from_path("README.md"),
            SourceLanguage::Unknown
        );
        assert!(!SourceLanguage::Unknown.is_tree_sitter_supported());
    }
}
