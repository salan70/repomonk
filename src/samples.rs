//! Built-in sample repositories used by the Home screen.

/// Language represented by a built-in sample repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleLanguage {
    TypeScript,
    Python,
    Java,
}

impl SampleLanguage {
    pub const fn label(self) -> &'static str {
        match self {
            Self::TypeScript => "TypeScript",
            Self::Python => "Python",
            Self::Java => "Java",
        }
    }
}

/// Progress mode that best demonstrates the sample repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleMode {
    Flow,
    Manual,
}

/// Metadata needed to open and present one sample repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleRepo {
    pub language: SampleLanguage,
    pub input: &'static str,
    pub mode: SampleMode,
    pub contract_revision: u16,
}

impl SampleRepo {
    pub const fn mode_label(self) -> &'static str {
        match self.mode {
            SampleMode::Flow => "flow",
            SampleMode::Manual => "manual",
        }
    }

    pub const fn search_label(self) -> &'static str {
        self.input
    }
}

/// The stable public sample set for the first-run experience.
pub const SAMPLE_REPOS: &[SampleRepo] = &[
    SampleRepo {
        language: SampleLanguage::TypeScript,
        input: "salan70/repomonk-sample-typescript",
        mode: SampleMode::Flow,
        contract_revision: 1,
    },
    SampleRepo {
        language: SampleLanguage::Python,
        input: "salan70/repomonk-sample-python",
        mode: SampleMode::Flow,
        contract_revision: 1,
    },
    SampleRepo {
        language: SampleLanguage::Java,
        input: "salan70/repomonk-sample-java",
        mode: SampleMode::Manual,
        contract_revision: 1,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_catalog_has_unique_github_inputs() {
        for (index, sample) in SAMPLE_REPOS.iter().enumerate() {
            assert!(sample.input.starts_with("salan70/"));
            assert!(sample.input.contains("repomonk-sample-"));
            assert_eq!(sample.contract_revision, 1);
            assert!(SAMPLE_REPOS[index + 1..]
                .iter()
                .all(|other| other.input != sample.input));
        }
    }
}
