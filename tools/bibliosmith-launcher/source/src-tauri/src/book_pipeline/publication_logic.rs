use super::publication::*;
use super::sha256_str;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub(super) const MAX_TRANSLATION_UNIT_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy)]
pub(super) struct MarkdownFence {
    marker: u8,
    width: usize,
}

pub(super) fn update_markdown_fence(line: &str, state: &mut Option<MarkdownFence>) -> bool {
    let bytes = line.as_bytes();
    let leading_spaces = bytes.iter().take_while(|byte| **byte == b' ').count();
    if leading_spaces > 3 || leading_spaces == bytes.len() {
        return false;
    }
    let marker = bytes[leading_spaces];
    if marker != b'`' && marker != b'~' {
        return false;
    }
    let width = bytes[leading_spaces..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    if width < 3 {
        return false;
    }

    if let Some(active) = state {
        let remainder = &bytes[leading_spaces + width..];
        if marker == active.marker
            && width >= active.width
            && remainder.iter().all(u8::is_ascii_whitespace)
        {
            *state = None;
            return true;
        }
        return false;
    }

    let remainder = &bytes[leading_spaces + width..];
    if marker == b'`' && remainder.contains(&b'`') {
        return false;
    }
    *state = Some(MarkdownFence { marker, width });
    true
}

pub(super) fn split_source_markdown(text: &str) -> SplitPlan {
    split_source_markdown_with_limit(text, MAX_TRANSLATION_UNIT_BYTES)
}

pub(super) fn split_source_markdown_with_limit(text: &str, max_unit_bytes: usize) -> SplitPlan {
    let lines: Vec<&str> = text.lines().collect();
    let mut fence = None;
    let heading_levels: Vec<Option<usize>> = lines
        .iter()
        .map(|line| {
            if update_markdown_fence(line, &mut fence) {
                return None;
            }
            if fence.is_some() {
                return None;
            }
            atx_heading_level(line)
        })
        .collect();
    let primary = heading_levels.iter().flatten().min().copied();
    let chapters = match primary {
        None => {
            if lines.iter().all(|line| line.trim().is_empty()) {
                Vec::new()
            } else {
                vec![build_chapter(1, "Chapter 1", 1, lines.len(), &lines)]
            }
        }
        Some(level) => split_at_headings(&lines, &heading_levels, level),
    };
    let chapters =
        bound_oversized_chapters(chapters, &lines, &heading_levels, primary, max_unit_bytes);
    let publication_sections = recover_publication_sections(&lines, &heading_levels, primary);
    SplitPlan {
        primary_heading_level: primary.unwrap_or(0),
        chapters,
        publication_sections,
    }
}

pub(super) fn markdown_page_anchor(line: &str) -> Option<u32> {
    let trimmed = line.trim();
    let value = trimmed
        .strip_prefix("<!-- page:")?
        .strip_suffix("-->")?
        .trim();
    value.parse().ok()
}

pub(super) fn source_pages_for_range(
    lines: &[&str],
    start_line: usize,
    end_line: usize,
) -> Vec<u32> {
    let mut pages = BTreeSet::new();
    let active_page = lines
        .iter()
        .take(start_line.saturating_sub(1))
        .rev()
        .find_map(|line| markdown_page_anchor(line));
    if let Some(page) = active_page {
        pages.insert(page);
    }
    for line in lines
        .iter()
        .take(end_line)
        .skip(start_line.saturating_sub(1))
    {
        if let Some(page) = markdown_page_anchor(line) {
            pages.insert(page);
        }
    }
    pages.into_iter().collect()
}

pub(super) fn source_character_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    let mut character = 0;
    for line in text.split_inclusive('\n') {
        character += line.chars().count();
        offsets.push(character);
    }
    offsets
}

pub(super) fn source_character_range(
    offsets: &[usize],
    start_line: usize,
    end_line: usize,
) -> (usize, usize) {
    let start = offsets
        .get(start_line.saturating_sub(1))
        .copied()
        .unwrap_or_default();
    let end = offsets
        .get(end_line)
        .copied()
        .or_else(|| offsets.last().copied())
        .unwrap_or(start);
    (start, end)
}

pub(super) fn apply_section_character_ranges(text: &str, sections: &mut [PublicationSection]) {
    let offsets = source_character_offsets(text);
    for section in sections {
        (section.source_start_character, section.source_end_character) =
            source_character_range(&offsets, section.source_start_line, section.source_end_line);
    }
}

pub(super) fn recover_publication_sections(
    lines: &[&str],
    heading_levels: &[Option<usize>],
    primary: Option<usize>,
) -> Vec<PublicationSection> {
    let headings = heading_levels
        .iter()
        .enumerate()
        .filter_map(|(index, level)| level.map(|level| (index, level)))
        .collect::<Vec<_>>();
    if headings.is_empty() {
        return if lines.iter().all(|line| line.trim().is_empty()) {
            Vec::new()
        } else {
            vec![PublicationSection {
                id: "section_001".into(),
                ordinal: 1,
                title: "Body".into(),
                short_title: "Body".into(),
                reader_title: None,
                reader_short_title: None,
                heading_level: 1,
                parent_id: None,
                role: PublicationRole::Bodymatter,
                kind: PublicationKind::Chapter,
                source_start_line: 1,
                source_end_line: lines.len(),
                source_start_character: 0,
                source_end_character: 0,
                source_pages: source_pages_for_range(lines, 1, lines.len()),
                source_files: Vec::new(),
                source_href: None,
                evidence: vec!["heading-free Markdown body".into()],
                confidence: 0.7,
                anomalies: Vec::new(),
            }]
        };
    }

    let mut sections = Vec::new();
    let first_heading = headings[0].0;
    if first_heading > 0
        && lines[..first_heading]
            .iter()
            .any(|line| !line.trim().is_empty() && markdown_page_anchor(line).is_none())
    {
        sections.push(PublicationSection {
            id: "section_001".into(),
            ordinal: 1,
            title: "Front Matter".into(),
            short_title: "Front Matter".into(),
            reader_title: None,
            reader_short_title: None,
            heading_level: primary.unwrap_or(1),
            parent_id: None,
            role: PublicationRole::Frontmatter,
            kind: PublicationKind::Frontmatter,
            source_start_line: 1,
            source_end_line: first_heading,
            source_start_character: 0,
            source_end_character: 0,
            source_pages: source_pages_for_range(lines, 1, first_heading),
            source_files: Vec::new(),
            source_href: None,
            evidence: vec!["non-empty content before the first normalized heading".into()],
            confidence: 0.7,
            anomalies: Vec::new(),
        });
    }

    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut backmatter_started = false;
    for (heading_index, (line_index, level)) in headings.iter().copied().enumerate() {
        while stack
            .last()
            .is_some_and(|(parent_level, _)| *parent_level >= level)
        {
            stack.pop();
        }
        let parent_id = stack.last().map(|(_, id)| id.clone());
        let title = heading_title(lines[line_index]);
        let (kind, explicit_role) = classify_publication_heading(&title, level, primary);
        if explicit_role == "backmatter" {
            backmatter_started = true;
        }
        let role = if backmatter_started {
            PublicationRole::Backmatter
        } else {
            explicit_role
        };
        let end_line = headings
            .iter()
            .skip(heading_index + 1)
            .find(|(_, candidate_level)| *candidate_level <= level)
            .map(|(next_index, _)| *next_index)
            .unwrap_or(lines.len());
        let ordinal = sections.len() + 1;
        let id = format!("section_{ordinal:03}");
        sections.push(PublicationSection {
            id: id.clone(),
            ordinal,
            short_title: title.clone(),
            title,
            reader_title: None,
            reader_short_title: None,
            heading_level: level,
            parent_id,
            role,
            kind,
            source_start_line: line_index + 1,
            source_end_line: end_line,
            source_start_character: 0,
            source_end_character: 0,
            source_pages: source_pages_for_range(lines, line_index + 1, end_line),
            source_files: Vec::new(),
            source_href: None,
            evidence: vec![format!(
                "normalized Markdown heading at line {}",
                line_index + 1
            )],
            confidence: 0.8,
            anomalies: Vec::new(),
        });
        stack.push((level, id));
    }
    sections
}

pub(super) fn classify_publication_heading(
    title: &str,
    level: usize,
    primary: Option<usize>,
) -> (PublicationKind, PublicationRole) {
    let folded = title.to_lowercase();
    if folded.contains("contents")
        || folded.contains("inhaltsverzeichnis")
        || title.contains("目录")
    {
        (PublicationKind::Contents, PublicationRole::Frontmatter)
    } else if folded == "title page" || folded == "titelblatt" || title.contains("题名页") {
        (PublicationKind::TitlePage, PublicationRole::Frontmatter)
    } else if folded.contains("copyright") || folded.contains("impressum") || title.contains("版权")
    {
        (PublicationKind::Copyright, PublicationRole::Frontmatter)
    } else if folded.contains("preface")
        || folded.contains("foreword")
        || folded.contains("vorwort")
        || title.contains("前言")
        || title.contains("序言")
    {
        (PublicationKind::Preface, PublicationRole::Frontmatter)
    } else if folded.contains("bibliography")
        || folded.contains("literaturverzeichnis")
        || title.contains("参考文献")
    {
        (PublicationKind::Bibliography, PublicationRole::Backmatter)
    } else if folded.contains("endnotes")
        || folded == "notes"
        || folded.contains("anmerkungen")
        || title.contains("注释")
    {
        (PublicationKind::Notes, PublicationRole::Backmatter)
    } else if folded.contains("appendix") || folded.contains("anhang") || title.contains("附录") {
        (PublicationKind::Appendix, PublicationRole::Backmatter)
    } else if folded.starts_with("part ")
        || folded.starts_with("teil ")
        || title.ends_with('部')
        || title.contains("部分")
    {
        (PublicationKind::Part, PublicationRole::Bodymatter)
    } else if primary == Some(level) {
        (PublicationKind::Chapter, PublicationRole::Bodymatter)
    } else {
        (PublicationKind::Section, PublicationRole::Bodymatter)
    }
}

pub(super) fn publication_sections_from_extracted_evidence(
    text: &str,
    evidence: &ExtractedPublicationEvidence,
) -> Result<Vec<PublicationSection>, String> {
    if evidence.schema != "publication-extraction-evidence-v2" {
        return Err("Extracted publication evidence has an unsupported schema.".into());
    }
    if evidence.sections.is_empty() {
        return Err("Extractor evidence contains no publication sections.".into());
    }
    let lines = text.lines().collect::<Vec<_>>();
    let source_document_paths = evidence
        .source_documents
        .iter()
        .map(|document| document.path.as_str())
        .collect::<BTreeSet<_>>();
    if evidence.source_documents.is_empty() {
        return Err("Extractor evidence contains no persisted source documents.".into());
    }
    let navigation_documents = evidence
        .source_documents
        .iter()
        .filter(|document| matches!(document.kind.as_str(), "epub_navigation" | "epub_ncx"))
        .collect::<Vec<_>>();
    if evidence.source_format == ExtractedSourceFormat::Epub {
        if navigation_documents.is_empty() {
            return Err("EPUB extractor evidence has no retained NAV or NCX document.".into());
        }
    } else if !navigation_documents.is_empty()
        || evidence
            .sections
            .iter()
            .any(|section| section.navigation_source_href.is_some())
    {
        return Err("Non-EPUB extractor evidence cannot declare EPUB navigation sources.".into());
    }
    if source_document_paths.len() != evidence.source_documents.len() {
        return Err("Extractor evidence has duplicate source-document paths.".into());
    }
    for document in &evidence.source_documents {
        let explicitly_unmapped =
            document.start_line == 0 && document.end_line == 0 && !document.anomalies.is_empty();
        if document.path.trim().is_empty()
            || document.path.starts_with('/')
            || document.path.contains('\\')
            || document
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "..")
            || (!explicitly_unmapped
                && (document.start_line == 0
                    || document.end_line < document.start_line
                    || document.end_line > lines.len()))
            || document.kind.trim().is_empty()
            || document.sha256.len() != 64
            || !document.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
            || document.pages.windows(2).any(|pages| pages[0] >= pages[1])
        {
            return Err(format!(
                "Extractor evidence has an invalid source document: {}",
                document.path
            ));
        }
    }
    let mut ids = BTreeSet::new();
    for section in &evidence.sections {
        if !is_canonical_publication_id(&section.id) {
            return Err(format!(
                "Extractor evidence has an invalid publication section ID: {}",
                section.id
            ));
        }
        if !ids.insert(section.id.as_str()) {
            return Err(format!(
                "Extractor evidence has a duplicate section ID: {}",
                section.id
            ));
        }
        if section.source_href.trim().is_empty()
            || section.confidence.is_some_and(|confidence| {
                !confidence.is_finite() || !(0.0..=1.0).contains(&confidence)
            })
        {
            return Err(format!(
                "Extractor evidence has invalid source identity or confidence: {}",
                section.id
            ));
        }
        if evidence.source_format == ExtractedSourceFormat::Epub
            && section
                .navigation_source_href
                .as_deref()
                .is_none_or(str::is_empty)
        {
            return Err(format!(
                "EPUB extractor evidence section has no authoritative navigation source: {}",
                section.id
            ));
        }
        if section.heading_level == 0 || section.heading_level > 6 {
            return Err(format!(
                "Extractor evidence has an invalid heading level: {} ({})",
                section.id, section.heading_level
            ));
        }
        if let Some(parent_id) = section.parent_id.as_deref() {
            let parent = evidence.sections.iter().find(|item| item.id == parent_id);
            if parent_id == section.id || parent.is_none() {
                return Err(format!(
                    "Extractor evidence has an invalid parent: {} ({parent_id})",
                    section.id
                ));
            }
            if parent.is_some_and(|parent| section.heading_level <= parent.heading_level) {
                return Err(format!(
                    "Extractor evidence parent depth does not contain its child: {} ({parent_id})",
                    section.id
                ));
            }
        }
        if section.source_start_line.is_some() != section.source_end_line.is_some() {
            return Err(format!(
                "Extractor evidence has an incomplete source range: {}",
                section.id
            ));
        }
        if let (Some(start), Some(end)) = (section.source_start_line, section.source_end_line) {
            if start == 0 || end < start || end > lines.len() {
                return Err(format!(
                    "Extractor evidence has an invalid source range: {} ({start}-{end})",
                    section.id
                ));
            }
            let normalized_title = normalized_structure_title(&section.title);
            let marker = format!(
                "<!-- bibliosmith-nav:{}:{} -->",
                section.id,
                sha256_str(&normalized_title)
            );
            let source_start = lines[start - 1];
            let title_matches = source_start.contains(&marker)
                || atx_heading_level(source_start).is_some_and(|_| {
                    normalized_structure_title(&heading_title(source_start)) == normalized_title
                });
            if normalized_title.is_empty() || !title_matches {
                return Err(format!(
                    "Extractor evidence title does not match its source range: {} ({})",
                    section.id, section.title
                ));
            }
        }
        if let Some(role) = section.role.as_deref() {
            PublicationRole::parse(role).ok_or_else(|| {
                format!(
                    "Extractor evidence has an unsupported role: {} ({role})",
                    section.id
                )
            })?;
        }
        if let Some(kind) = section.kind.as_deref() {
            PublicationKind::parse(kind).ok_or_else(|| {
                format!(
                    "Extractor evidence has an unsupported kind: {} ({kind})",
                    section.id
                )
            })?;
        }
        if !source_document_paths.is_empty() && section.source_files.is_empty() {
            return Err(format!(
                "Extractor evidence section has no source-document link: {}",
                section.id
            ));
        }
        if section.source_files.iter().any(|path| {
            !source_document_paths.is_empty() && !source_document_paths.contains(path.as_str())
        }) {
            return Err(format!(
                "Extractor evidence section references an unknown source document: {}",
                section.id
            ));
        }
    }
    for section in &evidence.sections {
        let mut visited = BTreeSet::new();
        let mut current = Some(section.id.as_str());
        while let Some(id) = current {
            if !visited.insert(id) {
                return Err(format!(
                    "Extractor evidence has a cyclic parent relationship at {}",
                    section.id
                ));
            }
            current = evidence
                .sections
                .iter()
                .find(|candidate| candidate.id == id)
                .and_then(|candidate| candidate.parent_id.as_deref());
        }
    }
    let headings = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            atx_heading_level(line).map(|level| (index, level, heading_title(line)))
        })
        .collect::<Vec<_>>();
    let mut matched_lines = Vec::new();
    let mut search_from = 0;
    for section in &evidence.sections {
        if let Some(start_line) = section.source_start_line {
            if start_line == 0 || start_line > lines.len().max(1) {
                return Err(format!(
                    "Extractor evidence has an invalid source line: {} ({})",
                    section.id, start_line
                ));
            }
            matched_lines.push(start_line - 1);
            continue;
        }
        let normalized = section
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let found = headings
            .iter()
            .enumerate()
            .skip(search_from)
            .find(|(_, (_, _, title))| {
                title.split_whitespace().collect::<Vec<_>>().join(" ") == normalized
            })
            .map(|(position, (line, _, _))| (position, *line));
        let Some((position, line)) = found else {
            return Err(format!(
                "Extractor structure target could not be matched to Markdown: {} ({})",
                section.id, section.source_href
            ));
        };
        matched_lines.push(line);
        search_from = position + 1;
    }
    if matched_lines.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err("Extractor evidence source ranges are out of publication order.".into());
    }

    for (index, section) in evidence.sections.iter().enumerate() {
        let start = matched_lines[index] + 1;
        let inferred_end = evidence
            .sections
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, candidate)| candidate.heading_level <= section.heading_level)
            .map(|(next, _)| matched_lines[next])
            .unwrap_or(lines.len());
        if section
            .source_end_line
            .is_some_and(|explicit_end| explicit_end != inferred_end)
        {
            return Err(format!(
                "Extractor evidence section end does not match the canonical publication boundary: {} (expected {inferred_end}).",
                section.id
            ));
        }
        let end = inferred_end;
        let mut expected_source_pages = source_pages_for_range(&lines, start, end);
        if expected_source_pages.is_empty() {
            expected_source_pages = evidence
                .source_documents
                .iter()
                .filter(|document| {
                    let explicitly_unmapped = document.start_line == 0
                        && document.end_line == 0
                        && !document.anomalies.is_empty();
                    !explicitly_unmapped && document.start_line <= end && start <= document.end_line
                })
                .flat_map(|document| document.pages.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        if section.source_pages != expected_source_pages {
            return Err(format!(
                "Extractor evidence section pages do not match its canonical source range: {}.",
                section.id
            ));
        }
        let mut expected_source_files = evidence
            .source_documents
            .iter()
            .filter(|document| {
                let explicitly_unmapped = document.start_line == 0
                    && document.end_line == 0
                    && !document.anomalies.is_empty();
                !explicitly_unmapped && document.start_line <= end && start <= document.end_line
            })
            .map(|document| document.path.as_str())
            .collect::<BTreeSet<_>>();
        if let Some(navigation_source_href) = section.navigation_source_href.as_deref() {
            let navigation_document = evidence
                .source_documents
                .iter()
                .find(|document| {
                    document.source_href == navigation_source_href
                        && matches!(document.kind.as_str(), "epub_navigation" | "epub_ncx")
                })
                .ok_or_else(|| {
                    format!(
                        "Extractor evidence section references an unknown navigation source: {} ({navigation_source_href})",
                        section.id
                    )
                })?;
            expected_source_files.insert(navigation_document.path.as_str());
        }
        let actual_source_files = section
            .source_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if actual_source_files.len() != section.source_files.len()
            || actual_source_files != expected_source_files
        {
            return Err(format!(
                "Extractor evidence section source documents do not match its source range: {}",
                section.id
            ));
        }
    }

    let id_map = evidence
        .sections
        .iter()
        .enumerate()
        .map(|(index, section)| (section.id.as_str(), format!("section_{:03}", index + 1)))
        .collect::<BTreeMap<_, _>>();
    let sections = evidence
        .sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            let ordinal = index + 1;
            let start = matched_lines[index];
            let inferred_end = evidence
                .sections
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, candidate)| candidate.heading_level <= section.heading_level)
                .map(|(next, _)| matched_lines[next])
                .unwrap_or(lines.len());
            let end = inferred_end;
            let (classified_kind, classified_role) =
                classify_publication_heading(&section.title, section.heading_level, Some(1));
            let role = section
                .role
                .as_deref()
                .and_then(PublicationRole::parse)
                .unwrap_or(classified_role);
            let kind = section
                .kind
                .as_deref()
                .and_then(PublicationKind::parse)
                .unwrap_or(classified_kind);
            let mut source_evidence = section.evidence.clone();
            source_evidence.push(format!(
                "{} structure target {} matched source line {}",
                evidence.source_format,
                section.source_href,
                start + 1
            ));
            if !section.source_files.is_empty() {
                source_evidence.push(format!(
                    "extractor source documents {}",
                    section.source_files.join(",")
                ));
            }
            PublicationSection {
                id: format!("section_{ordinal:03}"),
                ordinal,
                title: section.title.clone(),
                short_title: section.title.clone(),
                reader_title: None,
                reader_short_title: None,
                heading_level: section.heading_level,
                parent_id: section
                    .parent_id
                    .as_deref()
                    .and_then(|parent| id_map.get(parent).cloned()),
                role,
                kind,
                source_start_line: start + 1,
                source_end_line: end,
                source_start_character: 0,
                source_end_character: 0,
                source_pages: section.source_pages.clone(),
                source_files: section.source_files.clone(),
                source_href: Some(section.source_href.clone()),
                evidence: source_evidence,
                confidence: section.confidence.unwrap_or(1.0),
                anomalies: section.anomalies.clone(),
            }
        })
        .collect::<Vec<_>>();
    if let Some((section_id, finding)) = publication_tree_findings(&sections, lines.len()).first() {
        return Err(format!(
            "Compiled extractor publication tree is invalid: {section_id}: {finding}"
        ));
    }
    Ok(sections)
}

pub(super) fn is_internal_publication_title(title: &str) -> bool {
    let folded = title.trim().to_ascii_lowercase();
    if folded.is_empty() || folded == "continuation" || folded.starts_with("continuation ") {
        return true;
    }
    [
        "chapter_", "chapter-", "unit_", "unit-", "section_", "section-",
    ]
    .iter()
    .any(|prefix| {
        folded
            .strip_prefix(prefix)
            .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
    })
}

pub(super) fn publication_section_depth(
    section: &PublicationSection,
    by_id: &BTreeMap<&str, &PublicationSection>,
) -> usize {
    let mut depth = 1;
    let mut parent_id = section.parent_id.as_deref();
    let mut visited = BTreeSet::new();
    while let Some(id) = parent_id {
        if !visited.insert(id) {
            return usize::MAX;
        }
        let Some(parent) = by_id.get(id) else {
            return usize::MAX;
        };
        depth += 1;
        parent_id = parent.parent_id.as_deref();
    }
    depth
}

pub(super) fn publication_tree_findings(
    sections: &[PublicationSection],
    source_line_count: usize,
) -> Vec<(String, String)> {
    let mut findings = Vec::new();
    let mut ids = BTreeSet::new();
    let mut ordinals = BTreeSet::new();
    let by_id = sections
        .iter()
        .map(|section| (section.id.as_str(), section))
        .collect::<BTreeMap<_, _>>();
    for section in sections {
        if !is_canonical_publication_id(&section.id) {
            findings.push((section.id.clone(), "invalid publication section ID".into()));
        }
        if !ids.insert(section.id.as_str()) {
            findings.push((
                section.id.clone(),
                "duplicate publication section ID".into(),
            ));
        }
        if section.ordinal == 0
            || section.ordinal > sections.len()
            || !ordinals.insert(section.ordinal)
        {
            findings.push((section.id.clone(), "invalid publication ordinal".into()));
        }
        if !(1..=6).contains(&section.heading_level) {
            findings.push((section.id.clone(), "invalid heading level".into()));
        }
        if section.source_start_line == 0
            || section.source_end_line < section.source_start_line
            || section.source_end_line > source_line_count
        {
            findings.push((section.id.clone(), "invalid source range".into()));
        }
        let Some(parent_id) = section.parent_id.as_deref() else {
            continue;
        };
        if !is_canonical_publication_id(parent_id) {
            findings.push((section.id.clone(), "invalid publication parent ID".into()));
            continue;
        }
        let Some(parent) = by_id.get(parent_id) else {
            findings.push((section.id.clone(), "missing publication parent".into()));
            continue;
        };
        if parent_id == section.id {
            findings.push((section.id.clone(), "cyclic parent relationship".into()));
            continue;
        }
        if section.heading_level <= parent.heading_level {
            findings.push((
                section.id.clone(),
                "child heading is not deeper than its parent".into(),
            ));
        } else if section.heading_level > parent.heading_level + 1 {
            findings.push((
                section.id.clone(),
                "heading hierarchy jumps more than one level".into(),
            ));
        }
        if parent.ordinal >= section.ordinal {
            findings.push((
                section.id.clone(),
                "publication parent does not precede its child".into(),
            ));
        }
        if section.source_start_line < parent.source_start_line
            || section.source_end_line > parent.source_end_line
        {
            findings.push((
                section.id.clone(),
                "child source range escapes its parent".into(),
            ));
        }
    }
    for section in sections {
        let mut visited = BTreeSet::from([section.id.as_str()]);
        let mut parent_id = section.parent_id.as_deref();
        while let Some(id) = parent_id {
            if !visited.insert(id) {
                findings.push((section.id.clone(), "cyclic parent relationship".into()));
                break;
            }
            parent_id = by_id.get(id).and_then(|parent| parent.parent_id.as_deref());
        }
    }
    findings.sort();
    findings.dedup();
    findings
}

fn is_canonical_publication_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

pub(super) fn normalized_structure_title(title: &str) -> String {
    title
        .chars()
        .filter(|character| character.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

pub(super) fn printed_toc_match_counts(
    lines: &[&str],
    sections: &[PublicationSection],
) -> (usize, usize) {
    let Some(contents) = sections.iter().find(|section| section.kind == "contents") else {
        return (0, 0);
    };
    let known_titles = sections
        .iter()
        .filter(|section| section.id != contents.id)
        .map(|section| normalized_structure_title(&section.title))
        .filter(|title| !title.is_empty())
        .collect::<BTreeSet<_>>();
    let entries = lines
        .iter()
        .take(contents.source_end_line)
        .skip(contents.source_start_line.saturating_sub(1))
        .filter_map(|line| {
            let mut candidate = line.trim();
            if candidate.is_empty() || atx_heading_level(candidate).is_some() {
                return None;
            }
            candidate = candidate
                .trim_start_matches(|character: char| {
                    character.is_whitespace()
                        || character.is_ascii_digit()
                        || matches!(character, '-' | '*' | '+' | '.' | ')' | '(')
                })
                .trim();
            if let Some(open) = candidate.find('[') {
                if let Some(close) = candidate[open + 1..].find("](") {
                    candidate = &candidate[open + 1..open + 1 + close];
                }
            }
            let candidate = candidate
                .trim_end_matches(|character: char| {
                    character.is_whitespace()
                        || character.is_ascii_digit()
                        || matches!(character, '.' | '·' | '…')
                })
                .trim();
            let normalized = normalized_structure_title(candidate);
            (normalized.chars().count() >= 3).then_some(normalized)
        })
        .collect::<Vec<_>>();
    let unmatched = entries
        .iter()
        .filter(|entry| !known_titles.contains(*entry))
        .count();
    (entries.len(), unmatched)
}

pub(super) fn audit_publication_structure(
    text: &str,
    sections: &mut [PublicationSection],
    source: &str,
) -> PublicationStructureAudit {
    let lines = text.lines().collect::<Vec<_>>();
    let snapshots = sections.to_vec();
    let by_id = snapshots
        .iter()
        .map(|section| (section.id.as_str(), section))
        .collect::<BTreeMap<_, _>>();
    let mut anomalies = Vec::new();
    let mut role_counts = BTreeMap::new();
    let mut empty_nodes = 0usize;
    let mut suspected_cut_headings = 0usize;
    let mut maximum_depth = 0usize;
    let tree_findings = publication_tree_findings(&snapshots, lines.len());
    for section in sections.iter_mut() {
        for (_, finding) in tree_findings
            .iter()
            .filter(|(section_id, _)| section_id == &section.id)
        {
            if !section.anomalies.contains(finding) {
                section.anomalies.push(finding.clone());
            }
        }
        if !matches!(
            section.role.as_str(),
            "frontmatter" | "bodymatter" | "backmatter"
        ) {
            anomalies.push(format!(
                "{}: invalid publication role {}",
                section.id, section.role
            ));
        }
        if !matches!(
            section.kind.as_str(),
            "frontmatter"
                | "title_page"
                | "copyright"
                | "contents"
                | "preface"
                | "part"
                | "chapter"
                | "section"
                | "bibliography"
                | "notes"
                | "appendix"
        ) {
            anomalies.push(format!(
                "{}: invalid publication kind {}",
                section.id, section.kind
            ));
        }
        if !section.source_pages.is_empty()
            && !section
                .evidence
                .iter()
                .any(|evidence| evidence.starts_with("source page anchors "))
        {
            section.evidence.push(format!(
                "source page anchors {}",
                section
                    .source_pages
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        *role_counts
            .entry(section.role.as_str().to_string())
            .or_insert(0) += 1;
        if !matches!(
            section.role.as_str(),
            "frontmatter" | "bodymatter" | "backmatter"
        ) {
            section
                .anomalies
                .push("unsupported publication role".into());
        }
        let depth = publication_section_depth(section, &by_id);
        if depth != usize::MAX {
            maximum_depth = maximum_depth.max(depth);
        }
        if section.title.trim().is_empty() || is_internal_publication_title(&section.title) {
            section
                .anomalies
                .push("empty or internal publication title".into());
            empty_nodes += 1;
        }
        if section.source_start_line == 0
            || section.source_end_line < section.source_start_line
            || section.source_end_line > lines.len()
        {
            if !section
                .anomalies
                .iter()
                .any(|item| item == "invalid source range")
            {
                section.anomalies.push("invalid source range".into());
            }
            empty_nodes += 1;
        }
        if section.title.trim().chars().count() <= 1
            || section
                .title
                .trim_end()
                .chars()
                .last()
                .is_some_and(|character| matches!(character, '-' | '–' | '—'))
        {
            section.anomalies.push("suspected cut heading".into());
            suspected_cut_headings += 1;
        }
        if section.confidence < 0.6 {
            section
                .anomalies
                .push("low-confidence structure node".into());
        }
        anomalies.extend(
            section
                .anomalies
                .iter()
                .map(|finding| format!("{}: {finding}", section.id)),
        );
    }
    if sections.len() >= 8 && maximum_depth <= 1 {
        anomalies.push("publication hierarchy appears flattened".into());
    }
    if maximum_depth > 6 {
        anomalies.push(format!(
            "publication hierarchy exceeds supported depth: {maximum_depth}"
        ));
    }
    let (toc_entries, unmatched_toc_entries) = printed_toc_match_counts(&lines, sections);
    if unmatched_toc_entries >= 3 && unmatched_toc_entries * 3 > toc_entries {
        anomalies.push(format!(
            "printed contents has {unmatched_toc_entries} unmatched entries out of {toc_entries}"
        ));
    }
    let first_body = sections.iter().find(|section| section.role == "bodymatter");
    match first_body {
        None => anomalies.push("publication has no bodymatter section".into()),
        Some(section) => {
            let body_lines = lines
                .iter()
                .take(section.source_end_line)
                .skip(section.source_start_line.saturating_sub(1))
                .filter(|line| atx_heading_level(line).is_none())
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>();
            let body = body_lines
                .iter()
                .flat_map(|line| line.chars())
                .filter(|character| character.is_alphanumeric())
                .count();
            let author_only_shape = body < 40
                && body_lines.len() <= 1
                && body_lines.first().is_some_and(|line| {
                    !line.chars().any(|character| {
                        matches!(
                            character,
                            '.' | '。' | '!' | '！' | '?' | '？' | ';' | '；' | ':' | '：'
                        )
                    })
                });
            let title = section.title.trim();
            let title_is_isolated_number = !title.is_empty()
                && title
                    .chars()
                    .all(|character| character.is_ascii_digit() || character.is_whitespace());
            if author_only_shape || title_is_isolated_number || is_internal_publication_title(title)
            {
                anomalies.push(format!(
                    "{}: first bodymatter is not a meaningful titled body section",
                    section.id
                ));
            }
        }
    }
    let confidence = sections
        .iter()
        .map(|section| section.confidence)
        .fold(1.0_f64, f64::min);
    PublicationStructureAudit {
        status: if anomalies.is_empty() {
            "passed"
        } else {
            "failed"
        }
        .into(),
        source: source.into(),
        confidence,
        anomalies,
        node_count: sections.len(),
        maximum_depth,
        role_counts,
        empty_nodes,
        unmatched_toc_entries,
        suspected_cut_headings,
    }
}

pub(super) fn publication_structure_qa_json(
    audit: &PublicationStructureAudit,
) -> Result<String, String> {
    let report = serde_json::json!({
        "schema": "publication-structure-qa-v1",
        "status": audit.status,
        "source": audit.source,
        "confidence": audit.confidence,
        "nodeCount": audit.node_count,
        "maximumDepth": audit.maximum_depth,
        "roleCounts": audit.role_counts,
        "emptyNodes": audit.empty_nodes,
        "unmatchedTocEntries": audit.unmatched_toc_entries,
        "suspectedCutHeadings": audit.suspected_cut_headings,
        "anomalies": audit.anomalies,
    });
    Ok(serde_json::to_string_pretty(&report).map_err(|err| err.to_string())? + "\n")
}

pub(super) fn owning_publication_section_id(
    sections: &[PublicationSection],
    start_line: usize,
    end_line: usize,
) -> Option<String> {
    sections
        .iter()
        .filter(|section| {
            section.source_start_line <= start_line && end_line <= section.source_end_line
        })
        .max_by_key(|section| (section.heading_level, section.source_start_line))
        .map(|section| section.id.clone())
}

pub(super) fn footnote_definition_label(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("[^")?;
    let close = rest.find("]: ").or_else(|| rest.find("]:"))?;
    let label = &rest[..close];
    (!label.is_empty()).then_some(label)
}

pub(super) fn footnote_reference_labels(line: &str) -> Vec<&str> {
    let mut labels = Vec::new();
    let mut remainder = line;
    while let Some(open) = remainder.find("[^") {
        let after_open = &remainder[open + 2..];
        let Some(close) = after_open.find(']') else {
            break;
        };
        let label = &after_open[..close];
        let after = &after_open[close + 1..];
        if !label.is_empty() && !after.starts_with(':') {
            labels.push(label);
        }
        remainder = after;
    }
    labels
}

fn strip_html_comments(line: &str, in_comment: &mut bool) -> String {
    let mut remaining = line;
    let mut visible = String::new();
    loop {
        if *in_comment {
            let Some(end) = remaining.find("-->") else {
                return visible;
            };
            remaining = &remaining[end + 3..];
            *in_comment = false;
            continue;
        }
        let Some(start) = remaining.find("<!--") else {
            visible.push_str(remaining);
            return visible;
        };
        visible.push_str(&remaining[..start]);
        remaining = &remaining[start + 4..];
        *in_comment = true;
    }
}

fn strip_inline_code(line: &str, active_width: &mut Option<usize>) -> String {
    let bytes = line.as_bytes();
    let mut visible = String::with_capacity(line.len());
    let mut index = 0;
    while index < bytes.len() {
        if let Some(width) = *active_width {
            let closing = "`".repeat(width);
            let Some(relative_end) = line[index..].find(&closing) else {
                return visible;
            };
            index += relative_end + width;
            *active_width = None;
            continue;
        }
        if bytes[index] != b'`' {
            let character = line[index..].chars().next().expect("valid UTF-8 boundary");
            visible.push(character);
            index += character.len_utf8();
            continue;
        }
        let width = bytes[index..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        let closing = "`".repeat(width);
        let content_start = index + width;
        let Some(relative_end) = line[content_start..].find(&closing) else {
            *active_width = Some(width);
            return visible;
        };
        index = content_start + relative_end + width;
    }
    visible
}

fn semantic_markdown_lines(text: &str) -> Vec<String> {
    let mut in_comment = false;
    let mut fence = None;
    let mut inline_code_width = None;
    text.lines()
        .map(|line| {
            if fence.is_some() {
                update_markdown_fence(line, &mut fence);
                return String::new();
            }
            let without_comments = strip_html_comments(line, &mut in_comment);
            if update_markdown_fence(&without_comments, &mut fence) {
                String::new()
            } else {
                strip_inline_code(&without_comments, &mut inline_code_width)
            }
        })
        .collect()
}

pub(super) fn recover_publication_notes(
    text: &str,
    sections: &[PublicationSection],
    units: &[SplitChapter],
) -> Vec<PublicationNote> {
    let lines = text.lines().collect::<Vec<_>>();
    let semantic_lines = semantic_markdown_lines(text);
    let character_offsets = source_character_offsets(text);
    let mut label_order = Vec::new();
    let mut references: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut definitions: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (index, line) in semantic_lines.iter().enumerate() {
        if let Some(label) = footnote_definition_label(line) {
            let mut end = index + 1;
            for (continuation_index, continuation) in
                semantic_lines.iter().enumerate().skip(index + 1)
            {
                let indented = continuation.starts_with("    ") || continuation.starts_with('\t');
                let blank_before_indented = continuation.trim().is_empty()
                    && semantic_lines
                        .get(continuation_index + 1)
                        .is_some_and(|next| next.starts_with("    ") || next.starts_with('\t'));
                if indented || blank_before_indented {
                    end = continuation_index + 1;
                } else {
                    break;
                }
            }
            definitions.insert(label.to_string(), (index + 1, end));
        }
        for label in footnote_reference_labels(line) {
            if !references.contains_key(label) {
                label_order.push(label.to_string());
            }
            references
                .entry(label.to_string())
                .or_default()
                .push(index + 1);
        }
    }
    label_order
        .into_iter()
        .enumerate()
        .filter_map(|(index, label)| {
            let (source_line, source_end_line) = *definitions.get(&label)?;
            let ordinal = index + 1;
            let id = format!("note_{ordinal:03}");
            let publication_section_id =
                owning_publication_section_id(sections, source_line, source_end_line)?;
            let (source_start_character, source_end_character) =
                source_character_range(&character_offsets, source_line, source_end_line);
            let reference_lines = references.get(&label).cloned().unwrap_or_default();
            let count = reference_lines.len();
            let kind = if label.to_ascii_lowercase().starts_with("editor") {
                PublicationNoteKind::Editorial
            } else if label.to_ascii_lowercase().starts_with("end")
                || sections
                    .iter()
                    .any(|section| section.id == publication_section_id && section.kind == "notes")
            {
                PublicationNoteKind::Endnote
            } else {
                PublicationNoteKind::Footnote
            };
            let reference_ids = (1..=count)
                .map(|reference| format!("noteref_{id}_{reference:03}"))
                .collect::<Vec<_>>();
            let translation_unit_ids = units
                .iter()
                .filter(|unit| {
                    (unit.start_line <= source_line && source_line <= unit.end_line)
                        || reference_lines
                            .iter()
                            .any(|line| unit.start_line <= *line && *line <= unit.end_line)
                })
                .map(|unit| unit.id.clone())
                .collect();
            Some(PublicationNote {
                id: id.clone(),
                ordinal,
                source_label: label,
                kind,
                publication_section_id,
                source_start_line: source_line,
                source_end_line,
                source_start_character,
                source_end_character,
                source_anchor: format!("markdown-footnote-{id}"),
                source_pages: source_pages_for_range(&lines, source_line, source_end_line),
                source_files: Vec::new(),
                reference_source_lines: reference_lines,
                backlink_ids: reference_ids.clone(),
                reference_ids,
                translation_unit_ids,
                target_content_status: TargetContentStatus::PendingTranslation,
            })
        })
        .collect()
}

pub(super) fn validate_extracted_note_evidence(
    text: &str,
    evidence: &ExtractedPublicationEvidence,
    sections: &[PublicationSection],
    recovered: &mut [PublicationNote],
) -> Result<(), String> {
    if evidence.notes.len() != recovered.len() {
        return Err(format!(
            "Extractor Note evidence does not match canonical Markdown notes: expected {}, got {}.",
            recovered.len(),
            evidence.notes.len()
        ));
    }
    let lines = text.lines().collect::<Vec<_>>();
    let extracted_sections = evidence
        .sections
        .iter()
        .map(|section| (section.id.as_str(), section))
        .collect::<BTreeMap<_, _>>();
    let mut note_ids = BTreeSet::new();
    let mut labels = BTreeSet::new();
    for (contract, note) in evidence.notes.iter().zip(recovered.iter_mut()) {
        let kind = PublicationNoteKind::parse(&contract.kind).ok_or_else(|| {
            format!(
                "Extractor Note evidence has an unsupported kind: {} ({})",
                contract.id, contract.kind
            )
        })?;
        let extracted_section = extracted_sections
            .get(contract.publication_section_id.as_str())
            .ok_or_else(|| {
                format!(
                    "Extractor Note evidence references an unknown publication section: {}",
                    contract.id
                )
            })?;
        let deepest_extracted_section = evidence
            .sections
            .iter()
            .filter(|section| {
                section.source_start_line.is_some_and(|start| {
                    start <= contract.source_start_line
                        && section
                            .source_end_line
                            .is_some_and(|end| contract.source_end_line <= end)
                })
            })
            .max_by_key(|section| {
                (
                    section.heading_level,
                    section.source_start_line.unwrap_or(0),
                )
            });
        let extracted_start = extracted_section.source_start_line.unwrap_or(0);
        let extracted_end = extracted_section.source_end_line.unwrap_or(0);
        let source_files = contract
            .source_files
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let anchor_document = contract.source_anchor.split('#').next().unwrap_or_default();
        let expected_source_files = evidence
            .source_documents
            .iter()
            .filter(|document| {
                let definition_overlap = document.start_line <= contract.source_end_line
                    && contract.source_start_line <= document.end_line;
                let reference_overlap = contract
                    .reference_source_lines
                    .iter()
                    .any(|line| document.start_line <= *line && *line <= document.end_line);
                let source_anchor_match =
                    !anchor_document.is_empty() && document.source_href == anchor_document;
                definition_overlap || reference_overlap || source_anchor_match
            })
            .map(|document| document.path.as_str())
            .collect::<BTreeSet<_>>();
        let files_are_valid = source_files.len() == contract.source_files.len()
            && !source_files.is_empty()
            && source_files == expected_source_files;
        let mut expected_source_pages =
            source_pages_for_range(&lines, contract.source_start_line, contract.source_end_line);
        if expected_source_pages.is_empty() {
            expected_source_pages = evidence
                .source_documents
                .iter()
                .filter(|document| {
                    document.start_line <= contract.source_end_line
                        && contract.source_start_line <= document.end_line
                })
                .flat_map(|document| document.pages.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }
        let source_pages_are_bound = contract.source_pages == expected_source_pages;
        let reference_ids = (1..=contract.reference_source_lines.len())
            .map(|ordinal| format!("noteref_{}_{ordinal:03}", contract.id))
            .collect::<Vec<_>>();
        let references_match = contract.reference_source_lines.iter().all(|line| {
            lines.get(line.saturating_sub(1)).is_some_and(|source| {
                footnote_reference_labels(source).contains(&contract.source_label.as_str())
            })
        });
        let definition_matches = lines
            .get(contract.source_start_line.saturating_sub(1))
            .and_then(|line| footnote_definition_label(line))
            == Some(contract.source_label.as_str());
        let compiled_owner_id = owning_publication_section_id(
            sections,
            contract.source_start_line,
            contract.source_end_line,
        );
        let corresponding_compiled_section = sections.iter().find(|section| {
            section.source_start_line == extracted_start
                && section.source_end_line == extracted_end
                && section.heading_level == extracted_section.heading_level
                && normalized_structure_title(&section.title)
                    == normalized_structure_title(&extracted_section.title)
        });
        let owning_section_matches = deepest_extracted_section
            .is_some_and(|section| section.id == contract.publication_section_id)
            && compiled_owner_id.as_deref() == Some(note.publication_section_id.as_str())
            && corresponding_compiled_section.map_or_else(
                || {
                    sections
                        .iter()
                        .find(|section| section.id == note.publication_section_id)
                        .is_some_and(|section| {
                            section
                                .evidence
                                .iter()
                                .any(|item| item.starts_with("manual correction:"))
                        })
                },
                |section| section.id == note.publication_section_id,
            );
        if contract.id != note.id
            || contract.source_label != note.source_label
            || kind != note.kind
            || contract.source_start_line != note.source_start_line
            || contract.source_end_line != note.source_end_line
            || contract.reference_source_lines != note.reference_source_lines
            || contract.reference_ids != reference_ids
            || contract.reference_ids != note.reference_ids
            || contract.source_anchor.trim().is_empty()
            || contract.evidence.is_empty()
            || !contract.anomalies.is_empty()
            || !note_ids.insert(contract.id.as_str())
            || !labels.insert(contract.source_label.as_str())
            || contract.source_start_line == 0
            || contract.source_end_line < contract.source_start_line
            || contract.source_end_line > lines.len()
            || contract.reference_source_lines.is_empty()
            || contract
                .reference_source_lines
                .windows(2)
                .any(|pair| pair[0] > pair[1])
            || contract
                .source_pages
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
            || extracted_start == 0
            || contract.source_start_line < extracted_start
            || contract.source_end_line > extracted_end
            || !files_are_valid
            || !source_pages_are_bound
            || !references_match
            || !definition_matches
            || !owning_section_matches
        {
            return Err(format!(
                "Extractor Note evidence does not match canonical Markdown notes: {}.",
                contract.id
            ));
        }
        note.source_pages = contract.source_pages.clone();
        note.source_files.clone_from(&contract.source_files);
        note.source_anchor.clone_from(&contract.source_anchor);
    }
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PublicationNoteAudit {
    pub(super) schema: String,
    pub(super) status: String,
    pub(super) source_notes: usize,
    pub(super) source_references: usize,
    pub(super) recovered_notes: usize,
    pub(super) mapped_translation_units: usize,
    pub(super) target_content_ready: usize,
    pub(super) orphan_references: Vec<String>,
    pub(super) orphan_notes: Vec<String>,
    pub(super) duplicate_labels: Vec<String>,
}

pub(super) fn audit_publication_notes(
    text: &str,
    notes: &[PublicationNote],
) -> PublicationNoteAudit {
    let mut definitions: BTreeMap<String, usize> = BTreeMap::new();
    let mut references: BTreeMap<String, usize> = BTreeMap::new();
    for line in semantic_markdown_lines(text) {
        if let Some(label) = footnote_definition_label(&line) {
            *definitions.entry(label.to_string()).or_default() += 1;
        }
        for label in footnote_reference_labels(&line) {
            *references.entry(label.to_string()).or_default() += 1;
        }
    }
    let orphan_references = references
        .keys()
        .filter(|label| !definitions.contains_key(*label))
        .cloned()
        .collect::<Vec<_>>();
    let orphan_notes = definitions
        .keys()
        .filter(|label| !references.contains_key(*label))
        .cloned()
        .collect::<Vec<_>>();
    let duplicate_labels = definitions
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(label, _)| label.clone())
        .collect::<Vec<_>>();
    let passed = orphan_references.is_empty()
        && orphan_notes.is_empty()
        && duplicate_labels.is_empty()
        && notes.len() == definitions.len();
    PublicationNoteAudit {
        schema: "publication-note-qa-v1".into(),
        status: if passed { "passed" } else { "failed" }.into(),
        source_notes: definitions.len(),
        source_references: references.values().sum(),
        recovered_notes: notes.len(),
        mapped_translation_units: notes
            .iter()
            .flat_map(|note| note.translation_unit_ids.iter())
            .collect::<BTreeSet<_>>()
            .len(),
        target_content_ready: notes
            .iter()
            .filter(|note| note.target_content_status == "translated")
            .count(),
        orphan_references,
        orphan_notes,
        duplicate_labels,
    }
}

pub(super) fn publication_note_qa_json(audit: &PublicationNoteAudit) -> Result<String, String> {
    Ok(serde_json::to_string_pretty(audit).map_err(|err| err.to_string())? + "\n")
}

pub(super) fn bound_oversized_chapters(
    chapters: Vec<SplitChapter>,
    lines: &[&str],
    heading_levels: &[Option<usize>],
    primary: Option<usize>,
    max_unit_bytes: usize,
) -> Vec<SplitChapter> {
    let mut slices = Vec::new();
    for chapter in chapters {
        let start = chapter.start_line.saturating_sub(1);
        let end = chapter.end_line;
        if chapter.text.len() <= max_unit_bytes {
            slices.push((chapter.title, start, end));
            continue;
        }
        let deeper_level = heading_levels[start..end]
            .iter()
            .flatten()
            .copied()
            .filter(|level| primary.is_none_or(|primary| *level > primary))
            .min();
        let boundaries = if let Some(deeper_level) = deeper_level {
            heading_levels[start..end]
                .iter()
                .enumerate()
                .filter_map(|(offset, level)| {
                    (*level == Some(deeper_level)).then_some(start + offset)
                })
                .collect::<Vec<_>>()
        } else {
            paragraph_start_lines(lines, start, end)
        };
        if boundaries.is_empty() {
            slices.push((chapter.title, start, end));
            continue;
        }

        let mut unit_starts = vec![start];
        unit_starts.extend(boundaries.iter().skip(1).copied());
        let mut group_start = start;
        let mut group_title = chapter.title;
        for (position, &unit_start) in unit_starts.iter().enumerate().skip(1) {
            let unit_end = unit_starts.get(position + 1).copied().unwrap_or(end);
            if rendered_slice_len(&lines[group_start..unit_end]) > max_unit_bytes {
                slices.push((group_title, group_start, unit_start));
                group_start = unit_start;
                let title = heading_levels[unit_start]
                    .is_some()
                    .then(|| heading_title(lines[unit_start]))
                    .filter(|title| !title.is_empty());
                group_title = title.unwrap_or_else(|| "Continuation".into());
            }
        }
        slices.push((group_title, group_start, end));
    }

    let chapters = slices
        .into_iter()
        .enumerate()
        .map(|(index, (title, start, end))| {
            build_chapter(index + 1, &title, start + 1, end, &lines[start..end])
        })
        .collect();
    hard_bound_unstructured_text(chapters, max_unit_bytes)
}

pub(super) fn hard_bound_unstructured_text(
    chapters: Vec<SplitChapter>,
    max_unit_bytes: usize,
) -> Vec<SplitChapter> {
    let mut bounded = Vec::new();
    for chapter in chapters {
        let pieces = hard_bound_text(&chapter.text, max_unit_bytes);
        if pieces.len() == 1 {
            bounded.push(chapter);
            continue;
        }

        let mut start_line = chapter.start_line;
        for (piece_index, text) in pieces.into_iter().enumerate() {
            let newline_count = text.bytes().filter(|byte| *byte == b'\n').count();
            let end_line = if newline_count == 0 {
                start_line
            } else {
                start_line + newline_count - usize::from(text.ends_with('\n'))
            };
            let title = if piece_index == 0 {
                chapter.title.clone()
            } else {
                "Continuation".into()
            };
            bounded.push(build_chapter_from_text(
                bounded.len() + 1,
                &title,
                start_line,
                end_line,
                text,
            ));
            start_line += newline_count;
        }
    }

    for (index, chapter) in bounded.iter_mut().enumerate() {
        chapter.ordinal = index + 1;
        chapter.id = format!("chapter_{:03}", index + 1);
        chapter.blocks = paragraph_blocks_for_text(&chapter.text, chapter.start_line, &chapter.id);
    }
    bounded
}

pub(super) fn hard_bound_text(text: &str, max_unit_bytes: usize) -> Vec<String> {
    if text.len() <= max_unit_bytes {
        return vec![text.to_string()];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();
    for (atom, splittable) in structural_text_atoms(text) {
        let atom_pieces = if splittable {
            hard_bound_plain_text(atom, max_unit_bytes)
        } else {
            vec![atom.to_string()]
        };
        for piece in atom_pieces {
            if piece.len() > max_unit_bytes {
                if !current.is_empty() {
                    pieces.push(std::mem::take(&mut current));
                }
                pieces.push(piece);
            } else if current.len() + piece.len() <= max_unit_bytes {
                current.push_str(&piece);
            } else {
                pieces.push(std::mem::take(&mut current));
                current = piece;
            }
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    pieces
}

pub(super) fn hard_bound_plain_text(text: &str, max_unit_bytes: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut remaining = text;
    while remaining.len() > max_unit_bytes {
        let mut boundary = max_unit_bytes;
        while !remaining.is_char_boundary(boundary) {
            boundary -= 1;
        }
        let preferred = remaining[..boundary]
            .char_indices()
            .rev()
            .find_map(|(index, character)| {
                character
                    .is_whitespace()
                    .then_some(index + character.len_utf8())
            })
            .filter(|preferred| *preferred >= max_unit_bytes / 2);
        let split_at = preferred.unwrap_or(boundary);
        pieces.push(remaining[..split_at].to_string());
        remaining = &remaining[split_at..];
    }
    if !remaining.is_empty() {
        pieces.push(remaining.to_string());
    }
    pieces
}

pub(super) fn structural_text_atoms(text: &str) -> Vec<(&str, bool)> {
    let mut atoms = Vec::new();
    let mut fence = None;
    let mut plain_start = 0;
    let mut fence_start = None;
    let mut offset = 0;
    for segment in text.split_inclusive('\n') {
        let line = segment.strip_suffix('\n').unwrap_or(segment);
        let before = fence;
        let fence_line = update_markdown_fence(line, &mut fence);
        let next_offset = offset + segment.len();
        if fence_line && before.is_none() && fence.is_some() {
            if plain_start < offset {
                atoms.push((&text[plain_start..offset], true));
            }
            fence_start = Some(offset);
        } else if fence_line && before.is_some() && fence.is_none() {
            let start = fence_start.take().unwrap_or(offset);
            atoms.push((&text[start..next_offset], false));
            plain_start = next_offset;
        }
        offset = next_offset;
    }
    if let Some(start) = fence_start {
        atoms.push((&text[start..], false));
    } else if plain_start < text.len() {
        atoms.push((&text[plain_start..], true));
    }
    atoms
}

pub(super) fn rendered_slice_len(slice: &[&str]) -> usize {
    slice.iter().map(|line| line.len() + 1).sum()
}

pub(super) fn paragraph_start_lines(lines: &[&str], start: usize, end: usize) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut in_paragraph = false;
    let mut fence = None;
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        if update_markdown_fence(line, &mut fence) {
            if !in_paragraph {
                starts.push(index);
                in_paragraph = true;
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        if line.trim().is_empty() {
            in_paragraph = false;
        } else if !in_paragraph {
            starts.push(index);
            in_paragraph = true;
        }
    }
    starts
}

pub(super) fn atx_heading_level(line: &str) -> Option<usize> {
    let trimmed = line.trim_start();
    let hashes = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if (1..=6).contains(&hashes) {
        let rest = &trimmed[hashes..];
        if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') {
            return Some(hashes);
        }
    }
    None
}

pub(super) fn heading_title(line: &str) -> String {
    line.trim_start().trim_start_matches('#').trim().to_string()
}

pub(super) fn split_at_headings(
    lines: &[&str],
    heading_levels: &[Option<usize>],
    primary: usize,
) -> Vec<SplitChapter> {
    let boundaries: Vec<usize> = heading_levels
        .iter()
        .enumerate()
        .filter_map(|(index, level)| (*level == Some(primary)).then_some(index))
        .collect();
    let mut chapters = Vec::new();
    let mut ordinal = 0;
    let first = boundaries.first().copied().unwrap_or(0);
    let preamble_is_reader_visible = first > 0
        && semantic_markdown_lines(&lines[..first].join("\n"))
            .iter()
            .any(|line| !line.trim().is_empty());
    if preamble_is_reader_visible {
        ordinal += 1;
        chapters.push(build_chapter(
            ordinal,
            "Front Matter",
            1,
            first,
            &lines[..first],
        ));
    }
    for (position, &start) in boundaries.iter().enumerate() {
        let end = boundaries.get(position + 1).copied().unwrap_or(lines.len());
        ordinal += 1;
        let title = heading_title(lines[start]);
        let title = if title.is_empty() {
            format!("Chapter {ordinal}")
        } else {
            title
        };
        chapters.push(build_chapter(
            ordinal,
            &title,
            start + 1,
            end,
            &lines[start..end],
        ));
    }
    chapters
}

pub(super) fn build_chapter(
    ordinal: usize,
    title: &str,
    start_line: usize,
    end_line: usize,
    slice: &[&str],
) -> SplitChapter {
    let id = format!("chapter_{ordinal:03}");
    let text = if slice.is_empty() {
        String::new()
    } else {
        format!("{}\n", slice.join("\n"))
    };
    let blocks = paragraph_blocks(slice, start_line, &id);
    SplitChapter {
        ordinal,
        id,
        title: title.to_string(),
        start_line,
        end_line,
        text,
        blocks,
    }
}

pub(super) fn build_chapter_from_text(
    ordinal: usize,
    title: &str,
    start_line: usize,
    end_line: usize,
    text: String,
) -> SplitChapter {
    let id = format!("chapter_{ordinal:03}");
    let blocks = paragraph_blocks_for_text(&text, start_line, &id);
    SplitChapter {
        ordinal,
        id,
        title: title.to_string(),
        start_line,
        end_line,
        text,
        blocks,
    }
}

pub(super) fn paragraph_blocks_for_text(
    text: &str,
    slice_start_line: usize,
    chapter_id: &str,
) -> Vec<SplitBlock> {
    let lines = text.lines().collect::<Vec<_>>();
    paragraph_blocks(&lines, slice_start_line, chapter_id)
}

pub(super) fn paragraph_blocks(
    slice: &[&str],
    slice_start_line: usize,
    chapter_id: &str,
) -> Vec<SplitBlock> {
    let mut blocks = Vec::new();
    let mut index = 0;
    let mut ordinal = 0;
    while index < slice.len() {
        if slice[index].trim().is_empty() {
            index += 1;
            continue;
        }
        let start = index;
        while index < slice.len() && !slice[index].trim().is_empty() {
            index += 1;
        }
        ordinal += 1;
        let block_text = format!("{}\n", slice[start..index].join("\n"));
        blocks.push(SplitBlock {
            id: format!("{chapter_id}_block_{ordinal:03}"),
            start_line: slice_start_line + start,
            end_line: slice_start_line + index - 1,
            sha256: sha256_str(&block_text),
        });
    }
    blocks
}
