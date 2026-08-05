use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

macro_rules! string_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub(super) enum $name { $($variant),+ }

        impl $name {
            pub(super) const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $value),+ }
            }

            #[allow(dead_code)]
            pub(super) fn parse(value: &str) -> Option<Self> {
                match value { $($value => Some(Self::$variant)),+, _ => None }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl PartialEq<str> for $name {
            fn eq(&self, other: &str) -> bool { self.as_str() == other }
        }

        impl PartialEq<&str> for $name {
            fn eq(&self, other: &&str) -> bool { self.as_str() == *other }
        }

    };
}

string_enum!(PublicationRole {
    Frontmatter => "frontmatter",
    Bodymatter => "bodymatter",
    Backmatter => "backmatter",
});

string_enum!(ExtractedSourceFormat {
    Epub => "epub",
    Pdf => "pdf",
    Ocr => "ocr",
    Mineru => "mineru",
});

string_enum!(PublicationKind {
    Frontmatter => "frontmatter",
    TitlePage => "title_page",
    Copyright => "copyright",
    Contents => "contents",
    Preface => "preface",
    Part => "part",
    Chapter => "chapter",
    Section => "section",
    Bibliography => "bibliography",
    Notes => "notes",
    Appendix => "appendix",
});

string_enum!(PublicationNoteKind {
    Footnote => "footnote",
    Endnote => "endnote",
    Editorial => "editorial",
});

string_enum!(TargetContentStatus {
    PendingTranslation => "pending_translation",
    Translated => "translated",
});

pub(super) struct SplitPlan {
    pub(super) primary_heading_level: usize,
    pub(super) chapters: Vec<SplitChapter>,
    pub(super) publication_sections: Vec<PublicationSection>,
}

pub(super) struct SplitChapter {
    pub(super) ordinal: usize,
    pub(super) id: String,
    pub(super) title: String,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    pub(super) text: String,
    pub(super) blocks: Vec<SplitBlock>,
}

pub(super) struct SplitBlock {
    pub(super) id: String,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    pub(super) sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceMapDocument {
    pub(super) schema: String,
    pub(super) split_policy_version: String,
    pub(super) source_markdown_sha256: String,
    pub(super) source_path: String,
    pub(super) primary_heading_level: usize,
    pub(super) translation_units: Vec<SourceMapTranslationUnit>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceMapTranslationUnit {
    pub(super) id: String,
    pub(super) ordinal: usize,
    pub(super) publication_section_id: String,
    pub(super) source_start_line: usize,
    pub(super) source_end_line: usize,
    pub(super) source_start_character: usize,
    pub(super) source_end_character: usize,
    pub(super) section_start_line: usize,
    pub(super) section_end_line: usize,
    pub(super) section_start_character: usize,
    pub(super) section_end_character: usize,
    pub(super) source_unit_path: String,
    pub(super) source_unit_sha256: String,
    pub(super) source_anchor: Option<String>,
    #[serde(default)]
    pub(super) source_pages: Vec<u32>,
    pub(super) blocks: Vec<SourceMapBlock>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceMapBlock {
    pub(super) id: String,
    pub(super) source_start_line: usize,
    pub(super) source_end_line: usize,
    pub(super) source_start_character: usize,
    pub(super) source_end_character: usize,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicationSection {
    pub(super) id: String,
    pub(super) ordinal: usize,
    pub(super) title: String,
    pub(super) short_title: String,
    #[serde(default)]
    pub(super) reader_title: Option<String>,
    #[serde(default)]
    pub(super) reader_short_title: Option<String>,
    pub(super) heading_level: usize,
    pub(super) parent_id: Option<String>,
    pub(super) role: PublicationRole,
    pub(super) kind: PublicationKind,
    pub(super) source_start_line: usize,
    pub(super) source_end_line: usize,
    #[serde(default)]
    pub(super) source_start_character: usize,
    #[serde(default)]
    pub(super) source_end_character: usize,
    #[serde(default)]
    pub(super) source_pages: Vec<u32>,
    #[serde(default)]
    pub(super) source_files: Vec<String>,
    #[serde(default)]
    pub(super) source_href: Option<String>,
    #[serde(default)]
    pub(super) evidence: Vec<String>,
    #[serde(default = "default_structure_confidence")]
    pub(super) confidence: f64,
    #[serde(default)]
    pub(super) anomalies: Vec<String>,
}

fn default_structure_confidence() -> f64 {
    1.0
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicationMapDocument {
    pub(super) schema: String,
    pub(super) structure_policy_version: String,
    pub(super) source_markdown_sha256: String,
    pub(super) source_path: String,
    pub(super) sections: Vec<PublicationSection>,
    pub(super) notes: Vec<PublicationNote>,
    pub(super) audit: PublicationStructureAudit,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicationNote {
    pub(super) id: String,
    pub(super) ordinal: usize,
    pub(super) source_label: String,
    pub(super) kind: PublicationNoteKind,
    pub(super) publication_section_id: String,
    pub(super) source_start_line: usize,
    pub(super) source_end_line: usize,
    pub(super) source_start_character: usize,
    pub(super) source_end_character: usize,
    pub(super) source_anchor: String,
    pub(super) source_pages: Vec<u32>,
    #[serde(default)]
    pub(super) source_files: Vec<String>,
    pub(super) reference_source_lines: Vec<usize>,
    pub(super) reference_ids: Vec<String>,
    pub(super) backlink_ids: Vec<String>,
    pub(super) translation_unit_ids: Vec<String>,
    pub(super) target_content_status: TargetContentStatus,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicationStructureAudit {
    pub(super) status: String,
    pub(super) source: String,
    pub(super) confidence: f64,
    pub(super) anomalies: Vec<String>,
    pub(super) node_count: usize,
    pub(super) maximum_depth: usize,
    pub(super) role_counts: BTreeMap<String, usize>,
    pub(super) empty_nodes: usize,
    pub(super) unmatched_toc_entries: usize,
    pub(super) suspected_cut_headings: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExtractedPublicationEvidence {
    pub(super) schema: String,
    pub(super) source_format: ExtractedSourceFormat,
    #[serde(default)]
    pub(super) source_documents: Vec<ExtractedSourceDocument>,
    pub(super) sections: Vec<ExtractedPublicationSection>,
    #[serde(default)]
    pub(super) notes: Vec<ExtractedPublicationNote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExtractedPublicationNote {
    pub(super) id: String,
    pub(super) source_label: String,
    pub(super) kind: String,
    pub(super) publication_section_id: String,
    pub(super) source_start_line: usize,
    pub(super) source_end_line: usize,
    #[serde(default)]
    pub(super) source_pages: Vec<u32>,
    #[serde(default)]
    pub(super) source_files: Vec<String>,
    #[serde(default)]
    pub(super) reference_source_lines: Vec<usize>,
    #[serde(default)]
    pub(super) reference_ids: Vec<String>,
    pub(super) source_anchor: String,
    #[serde(default)]
    pub(super) evidence: Vec<String>,
    #[serde(default)]
    pub(super) anomalies: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExtractedSourceDocument {
    pub(super) path: String,
    pub(super) start_line: usize,
    pub(super) end_line: usize,
    #[serde(default)]
    pub(super) pages: Vec<u32>,
    pub(super) kind: String,
    pub(super) sha256: String,
    #[serde(default)]
    pub(super) anomalies: Vec<String>,
    #[serde(default)]
    pub(super) source_href: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ExtractedPublicationSection {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) parent_id: Option<String>,
    pub(super) heading_level: usize,
    pub(super) source_href: String,
    #[serde(default)]
    pub(super) navigation_source_href: Option<String>,
    #[serde(default)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) kind: Option<String>,
    #[serde(default)]
    pub(super) source_start_line: Option<usize>,
    #[serde(default)]
    pub(super) source_end_line: Option<usize>,
    #[serde(default)]
    pub(super) source_pages: Vec<u32>,
    #[serde(default)]
    pub(super) source_files: Vec<String>,
    #[serde(default)]
    pub(super) evidence: Vec<String>,
    #[serde(default)]
    pub(super) confidence: Option<f64>,
    #[serde(default)]
    pub(super) anomalies: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PublicationStructureCorrection {
    pub(super) schema: String,
    pub(super) source_markdown_sha256: String,
    pub(super) recovered_structure_sha256: String,
    pub(super) reason: String,
    pub(super) sections: Vec<PublicationSection>,
}
