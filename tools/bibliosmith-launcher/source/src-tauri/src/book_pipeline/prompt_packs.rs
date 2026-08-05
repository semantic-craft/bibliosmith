//! Translation Prompt Packs: immutable built-ins, append-only local copies, defaults,
//! and ephemeral compilation. Private source text is accepted only by compile methods
//! and never enters the store types below.

use super::*;

const BUILTIN_CATALOG_JSON: &str = include_str!("../../resources/translation-prompt-packs.json");
const PROMPT_PACK_STORE_SCHEMA: &str = "translation-prompt-pack-store-v1";

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackReference {
    pub pack_id: String,
    pub revision_id: String,
    pub content_sha256: String,
}

impl PromptPackReference {
    pub fn new(pack_id: &str, revision_id: &str, content_sha256: &str) -> Self {
        Self {
            pack_id: pack_id.into(),
            revision_id: revision_id.into(),
            content_sha256: content_sha256.into(),
        }
    }

    pub fn from_revision(revision: &PromptPackRevision) -> Self {
        Self {
            pack_id: revision.pack_id.clone(),
            revision_id: revision.revision_id.clone(),
            content_sha256: revision.content_sha256.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptStageTemplate {
    pub stage_id: String,
    pub label: String,
    pub template: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackRevision {
    pub schema: String,
    pub pack_id: String,
    pub revision_id: String,
    pub content_sha256: String,
    pub display_name: String,
    pub executor: String,
    pub source_language: String,
    pub target_language: String,
    pub cost_hint: String,
    pub source: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skill_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub excluded_responsibilities: Vec<String>,
    pub stages: Vec<PromptStageTemplate>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackDefinition {
    pub pack_id: String,
    pub kind: String,
    pub summary: String,
    pub revisions: Vec<PromptPackRevision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<String>,
}

impl PromptPackDefinition {
    pub fn latest_revision(&self) -> Option<&PromptPackRevision> {
        self.revisions.last()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackCatalog {
    pub schema: String,
    pub packs: Vec<PromptPackDefinition>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PromptPackRevisionDraft {
    pub pack_id: String,
    pub display_name: String,
    pub stages: Vec<PromptStageTemplate>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExpertPromptHandoff {
    pub prompt_pack_reference: PromptPackReference,
    pub source_language: String,
    pub target_language: String,
    pub context_policy: String,
    pub required_skill_ids: Vec<String>,
    pub skill_dependency_versions: serde_json::Value,
    pub required_evidence: Vec<String>,
    pub excluded_responsibilities: Vec<String>,
    pub actual_prompt: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackStageDiff {
    pub stage_id: String,
    pub before_template: Option<String>,
    pub after_template: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackRevisionDiff {
    pub before: PromptPackReference,
    pub after: PromptPackReference,
    pub display_name_changed: bool,
    pub source_changed: bool,
    pub stages: Vec<PromptPackStageDiff>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PromptPackStoreState {
    schema: String,
    #[serde(default)]
    custom_packs: Vec<PromptPackDefinition>,
    #[serde(default)]
    defaults: BTreeMap<String, PromptPackReference>,
}

impl Default for PromptPackStoreState {
    fn default() -> Self {
        Self {
            schema: PROMPT_PACK_STORE_SCHEMA.into(),
            custom_packs: Vec::new(),
            defaults: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PromptPackStore {
    path: PathBuf,
}

impl PromptPackStore {
    pub fn default() -> Result<Self, String> {
        Ok(Self {
            path: default_state_dir()?.join("prompt-packs.json"),
        })
    }

    #[cfg(test)]
    pub fn for_test(root: &Path) -> Self {
        Self {
            path: root.join("state").join("prompt-packs.json"),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn list(&self) -> Result<PromptPackCatalog, String> {
        let mut catalog = builtin_prompt_pack_catalog()?;
        catalog.packs.extend(
            self.load()?
                .custom_packs
                .into_iter()
                .filter(|pack| pack.deleted_at.is_none()),
        );
        Ok(catalog)
    }

    pub fn resolve_revision(
        &self,
        reference: &PromptPackReference,
        executor: &str,
    ) -> Result<PromptPackRevision, String> {
        let builtins = builtin_prompt_pack_catalog()?;
        let state = self.load()?;
        let revision = builtins
            .packs
            .iter()
            .chain(state.custom_packs.iter())
            .find(|pack| pack.pack_id == reference.pack_id)
            .and_then(|pack| {
                pack.revisions
                    .iter()
                    .find(|revision| revision.revision_id == reference.revision_id)
            })
            .ok_or_else(|| "prompt_pack_revision_not_found".to_string())?;
        validate_prompt_pack_revision(revision)?;
        if revision.content_sha256 != reference.content_sha256 {
            return Err("prompt_pack_content_hash_mismatch".into());
        }
        if revision.executor != executor {
            return Err("prompt_pack_executor_mismatch".into());
        }
        Ok(revision.clone())
    }

    pub fn copy_builtin(
        &self,
        source_reference: &PromptPackReference,
        display_name: &str,
    ) -> Result<PromptPackDefinition, String> {
        let source = resolve_builtin_revision(source_reference)?;
        let display_name = required_trimmed(display_name, "displayName")?;
        let mut state = self.load()?;
        let identity = sha256_str(&format!(
            "{}:{}:{}:{}",
            source.pack_id,
            source.revision_id,
            display_name,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let pack_id = format!("local.{}", &identity[..20]);
        let mut revision = source.clone();
        revision.pack_id = pack_id.clone();
        revision.revision_id = next_local_revision_id();
        revision.display_name = display_name.into();
        revision.source = serde_json::json!({
            "kind": "local-copy",
            "sourcePackId": source.pack_id,
            "sourceRevisionId": source.revision_id,
            "sourceContentSha256": source.content_sha256,
        });
        revision.content_sha256 = prompt_pack_content_sha256(&revision)?;
        let definition = PromptPackDefinition {
            pack_id: pack_id.clone(),
            kind: "custom".into(),
            summary: format!("由“{}”复制的本地方案", source.display_name),
            revisions: vec![revision],
            deleted_at: None,
        };
        state.custom_packs.push(definition.clone());
        self.save(&state)?;
        Ok(definition)
    }

    pub fn save_custom_revision(
        &self,
        draft: PromptPackRevisionDraft,
    ) -> Result<PromptPackRevision, String> {
        let mut state = self.load()?;
        let pack = state
            .custom_packs
            .iter_mut()
            .find(|pack| pack.pack_id == draft.pack_id && pack.deleted_at.is_none())
            .ok_or_else(|| "custom_prompt_pack_not_found".to_string())?;
        let current = pack
            .latest_revision()
            .cloned()
            .ok_or_else(|| "custom_prompt_pack_has_no_revision".to_string())?;
        let display_name = required_trimmed(&draft.display_name, "displayName")?;
        validate_editable_stages(&current.stages, &draft.stages)?;
        let mut revision = current;
        revision.revision_id = next_local_revision_id();
        revision.display_name = display_name.into();
        revision.stages = draft.stages;
        revision.content_sha256 = prompt_pack_content_sha256(&revision)?;
        validate_prompt_pack_revision(&revision)?;
        pack.revisions.push(revision.clone());
        self.save(&state)?;
        Ok(revision)
    }

    pub fn delete_custom(&self, pack_id: &str) -> Result<(), String> {
        if pack_id.starts_with("builtin.") {
            return Err("builtin_prompt_pack_is_read_only".into());
        }
        let mut state = self.load()?;
        if state
            .defaults
            .values()
            .any(|value| value.pack_id == pack_id)
        {
            return Err("default_prompt_pack_cannot_be_deleted".into());
        }
        let pack = state
            .custom_packs
            .iter_mut()
            .find(|pack| pack.pack_id == pack_id && pack.deleted_at.is_none())
            .ok_or_else(|| "custom_prompt_pack_not_found".to_string())?;
        pack.deleted_at = Some(now_label());
        self.save(&state)
    }

    pub fn resolve_default(
        &self,
        executor: &str,
        source_language: &str,
        target_language: &str,
    ) -> Result<PromptPackReference, String> {
        let key = default_key(executor, source_language, target_language);
        let reference = self
            .load()?
            .defaults
            .get(&key)
            .cloned()
            .unwrap_or_else(|| builtin_default(executor));
        self.resolve_revision(&reference, executor)?;
        Ok(reference)
    }

    pub fn set_default(
        &self,
        executor: &str,
        source_language: &str,
        target_language: &str,
        reference: PromptPackReference,
    ) -> Result<(), String> {
        let revision = self.resolve_revision(&reference, executor)?;
        if revision.source_language != source_language
            || revision.target_language != target_language
        {
            return Err("prompt_pack_language_mismatch".into());
        }
        let mut state = self.load()?;
        state.defaults.insert(
            default_key(executor, source_language, target_language),
            reference,
        );
        self.save(&state)
    }

    pub fn compile_expert_handoff(
        &self,
        reference: &PromptPackReference,
        source_sample: &str,
        glossary_entries: &[String],
    ) -> Result<ExpertPromptHandoff, String> {
        let revision = self.resolve_revision(reference, "expert-agent")?;
        let stages = revision
            .stages
            .iter()
            .map(|stage| format!("## {}\n{}", stage.label, stage.template))
            .collect::<Vec<_>>()
            .join("\n\n");
        let glossary = if glossary_entries.is_empty() {
            "（本次没有术语注入）".into()
        } else {
            glossary_entries.join("\n")
        };
        let actual_prompt = format!(
            "# TRANSLATION PROMPT PACK\n{}\n\n# STAGES\n{}\n\n# CONTEXT POLICY\n{}\n\n# CURRENT SOURCE SAMPLE\n{}\n\n# GLOSSARY\n{}\n\n# EXECUTOR SAFETY\n占位符、标题、段落边界、术语约束和私人文本边界由 BiblioSmith 执行器拥有，任何阶段不得覆盖。",
            revision.display_name,
            stages,
            revision.context_policy.as_deref().unwrap_or("仅使用最小必要上下文。"),
            source_sample,
            glossary,
        );
        Ok(ExpertPromptHandoff {
            prompt_pack_reference: reference.clone(),
            source_language: revision.source_language,
            target_language: revision.target_language,
            context_policy: revision.context_policy.unwrap_or_default(),
            required_skill_ids: revision.required_skill_ids,
            skill_dependency_versions: revision
                .source
                .get("skillVersions")
                .cloned()
                .unwrap_or_else(|| serde_json::json!({})),
            required_evidence: revision.required_evidence,
            excluded_responsibilities: revision.excluded_responsibilities,
            actual_prompt,
        })
    }

    pub fn validate_expert_receipt(
        &self,
        reference: &PromptPackReference,
        receipt: &serde_json::Value,
        handoff_sha256: &str,
    ) -> Result<(), String> {
        let revision = self.resolve_revision(reference, "expert-agent")?;
        if receipt.get("schema").and_then(serde_json::Value::as_str)
            != Some("translation-prompt-pack-receipt-v1")
            || receipt
                .get("translationHandoffSha256")
                .and_then(serde_json::Value::as_str)
                != Some(handoff_sha256)
            || receipt.get("promptPackReference") != serde_json::to_value(reference).ok().as_ref()
        {
            return Err("invalid_translation_prompt_pack_receipt".into());
        }
        let stage_evidence = receipt
            .get("stageEvidence")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| "missing_translation_stage_evidence".to_string())?;
        for evidence_type in &revision.required_evidence {
            let digest = stage_evidence
                .get(evidence_type)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| format!("missing_translation_stage_evidence:{evidence_type}"))?;
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!(
                    "invalid_translation_stage_evidence:{evidence_type}"
                ));
            }
        }
        if revision.pack_id == "builtin.full-quality-loop" {
            let translator = receipt
                .get("translatorId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let reviewer = receipt
                .get("independentReviewerId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if translator.is_empty() || reviewer.is_empty() || translator == reviewer {
                return Err("independent_review_not_isolated".into());
            }
            if receipt
                .get("latestReviewOpenIssueCount")
                .and_then(serde_json::Value::as_u64)
                .is_none_or(|count| count != 0)
            {
                return Err("expert_gate_open_issues".into());
            }
            let families = receipt
                .get("defectFamilies")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| "defect_family_evidence_missing".to_string())?;
            for family in families {
                let complete = family.get("status").and_then(serde_json::Value::as_str)
                    == Some("closed")
                    && ["candidateScanSha256", "repairSha256", "recheckSha256"]
                        .iter()
                        .all(|key| {
                            family
                                .get(*key)
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|digest| {
                                    digest.len() == 64
                                        && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                                })
                        });
                if !complete {
                    return Err("defect_family_not_closed".into());
                }
            }
        }
        Ok(())
    }

    pub fn diff_revisions(
        &self,
        before: &PromptPackReference,
        after: &PromptPackReference,
    ) -> Result<PromptPackRevisionDiff, String> {
        let before_revision = self.resolve_revision_without_executor(before)?;
        let after_revision = self.resolve_revision_without_executor(after)?;
        if before_revision.pack_id != after_revision.pack_id {
            return Err("prompt_pack_diff_requires_same_pack".into());
        }
        let before_stages = before_revision
            .stages
            .iter()
            .map(|stage| (stage.stage_id.as_str(), stage.template.as_str()))
            .collect::<BTreeMap<_, _>>();
        let after_stages = after_revision
            .stages
            .iter()
            .map(|stage| (stage.stage_id.as_str(), stage.template.as_str()))
            .collect::<BTreeMap<_, _>>();
        let stage_ids = before_stages
            .keys()
            .chain(after_stages.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        Ok(PromptPackRevisionDiff {
            before: before.clone(),
            after: after.clone(),
            display_name_changed: before_revision.display_name != after_revision.display_name,
            source_changed: before_revision.source != after_revision.source,
            stages: stage_ids
                .into_iter()
                .filter_map(|stage_id| {
                    let before_template = before_stages.get(stage_id).map(|value| (*value).into());
                    let after_template = after_stages.get(stage_id).map(|value| (*value).into());
                    (before_template != after_template).then_some(PromptPackStageDiff {
                        stage_id: stage_id.into(),
                        before_template,
                        after_template,
                    })
                })
                .collect(),
        })
    }

    fn resolve_revision_without_executor(
        &self,
        reference: &PromptPackReference,
    ) -> Result<PromptPackRevision, String> {
        let builtins = builtin_prompt_pack_catalog()?;
        let state = self.load()?;
        let revision = builtins
            .packs
            .iter()
            .chain(state.custom_packs.iter())
            .find(|pack| pack.pack_id == reference.pack_id)
            .and_then(|pack| {
                pack.revisions
                    .iter()
                    .find(|revision| revision.revision_id == reference.revision_id)
            })
            .ok_or_else(|| "prompt_pack_revision_not_found".to_string())?;
        validate_prompt_pack_revision(revision)?;
        if revision.content_sha256 != reference.content_sha256 {
            return Err("prompt_pack_content_hash_mismatch".into());
        }
        Ok(revision.clone())
    }

    fn load(&self) -> Result<PromptPackStoreState, String> {
        if !self.path.is_file() {
            return Ok(PromptPackStoreState::default());
        }
        let state: PromptPackStoreState =
            serde_json::from_str(&fs::read_to_string(&self.path).map_err(|err| err.to_string())?)
                .map_err(|err| err.to_string())?;
        if state.schema != PROMPT_PACK_STORE_SCHEMA {
            return Err("unsupported_prompt_pack_store_schema".into());
        }
        Ok(state)
    }

    fn save(&self, state: &PromptPackStoreState) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "Prompt Pack store path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
        let text = serde_json::to_string_pretty(state).map_err(|err| err.to_string())?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|err| err.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|err| err.to_string())?;
        temporary
            .persist(&self.path)
            .map_err(|err| err.error.to_string())?;
        Ok(())
    }
}

pub fn builtin_prompt_pack_catalog() -> Result<PromptPackCatalog, String> {
    let catalog: PromptPackCatalog =
        serde_json::from_str(BUILTIN_CATALOG_JSON).map_err(|err| err.to_string())?;
    if catalog.schema != "translation-prompt-pack-catalog-v1" {
        return Err("unsupported_prompt_pack_catalog_schema".into());
    }
    for pack in &catalog.packs {
        if pack.kind != "builtin" || pack.pack_id.is_empty() || pack.revisions.is_empty() {
            return Err("invalid_builtin_prompt_pack".into());
        }
        for revision in &pack.revisions {
            validate_prompt_pack_revision(revision)?;
        }
    }
    Ok(catalog)
}

pub fn validate_prompt_pack_revision(revision: &PromptPackRevision) -> Result<(), String> {
    if revision.schema != "translation-prompt-pack-revision-v1"
        || revision.pack_id.is_empty()
        || revision.revision_id.is_empty()
        || revision.display_name.trim().is_empty()
        || !matches!(revision.executor.as_str(), "programmatic" | "expert-agent")
        || revision.source_language != "auto"
        || revision.target_language != "zh-Hans"
        || revision.stages.is_empty()
    {
        return Err("invalid_prompt_pack_revision".into());
    }
    let mut ids = BTreeSet::new();
    for stage in &revision.stages {
        if stage.stage_id.trim().is_empty()
            || stage.label.trim().is_empty()
            || stage.template.trim().is_empty()
            || !ids.insert(stage.stage_id.as_str())
        {
            return Err("invalid_prompt_pack_stages".into());
        }
    }
    let computed_content_sha256 = prompt_pack_content_sha256(revision)?;
    if computed_content_sha256 != revision.content_sha256 {
        return Err(format!(
            "prompt_pack_content_hash_mismatch:{}:{}:{}",
            revision.pack_id, revision.content_sha256, computed_content_sha256
        ));
    }
    Ok(())
}

fn resolve_builtin_revision(reference: &PromptPackReference) -> Result<PromptPackRevision, String> {
    let catalog = builtin_prompt_pack_catalog()?;
    let revision = catalog
        .packs
        .iter()
        .find(|pack| pack.pack_id == reference.pack_id)
        .and_then(|pack| {
            pack.revisions
                .iter()
                .find(|revision| revision.revision_id == reference.revision_id)
        })
        .ok_or_else(|| "builtin_prompt_pack_revision_not_found".to_string())?;
    if revision.content_sha256 != reference.content_sha256 {
        return Err("prompt_pack_content_hash_mismatch".into());
    }
    Ok(revision.clone())
}

fn prompt_pack_content_sha256(revision: &PromptPackRevision) -> Result<String, String> {
    let mut value = serde_json::to_value(revision).map_err(|err| err.to_string())?;
    value
        .as_object_mut()
        .ok_or_else(|| "invalid_prompt_pack_revision".to_string())?
        .remove("contentSha256");
    Ok(sha256_str(&canonical_json(&value)))
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".into(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::String(value) => serde_json::to_string(value).unwrap_or_default(),
        serde_json::Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",")
        ),
        serde_json::Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort();
            format!(
                "{{{}}}",
                keys.into_iter()
                    .map(|key| format!(
                        "{}:{}",
                        serde_json::to_string(key).unwrap_or_default(),
                        canonical_json(&values[key])
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
    }
}

fn validate_editable_stages(
    current: &[PromptStageTemplate],
    edited: &[PromptStageTemplate],
) -> Result<(), String> {
    let current_ids = current
        .iter()
        .map(|stage| stage.stage_id.as_str())
        .collect::<Vec<_>>();
    let edited_ids = edited
        .iter()
        .map(|stage| stage.stage_id.as_str())
        .collect::<Vec<_>>();
    if current_ids != edited_ids
        || edited
            .iter()
            .any(|stage| stage.label.trim().is_empty() || stage.template.trim().is_empty())
    {
        return Err("prompt_pack_executor_contract_is_read_only".into());
    }
    Ok(())
}

fn required_trimmed<'a>(value: &'a str, key: &str) -> Result<&'a str, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{key} is required"));
    }
    Ok(value)
}

fn default_key(executor: &str, source_language: &str, target_language: &str) -> String {
    format!("{executor}:{source_language}:{target_language}")
}

fn builtin_default(executor: &str) -> PromptPackReference {
    match executor {
        "programmatic" => PromptPackReference::new(
            "builtin.structure-fidelity",
            "2026.08.05-1",
            "fb5dae8c498d46a1a3501acd0d6b00645b7dfe4c5c797e8e71732482c5a0c26f",
        ),
        "expert-agent" => PromptPackReference::new(
            "builtin.context-backtracking",
            "2026.08.05-1",
            "48c5907dda2fe67c29bfb84ea8690a64a9986d5a9f06a6b4fe42d08b92bd8833",
        ),
        _ => PromptPackReference::new("", "", ""),
    }
}

pub fn default_prompt_pack_reference_for_mode(translation_mode: &str) -> PromptPackReference {
    builtin_default(if translation_mode == TRANSLATION_MODE_EXPERT {
        "expert-agent"
    } else {
        "programmatic"
    })
}

pub fn four_dimension_prompt_pack_reference() -> PromptPackReference {
    PromptPackReference::new(
        "builtin.four-dimension-refinement",
        "2026.08.05-1",
        "fd6cdf2a208280776de49bedb863186749ff3dad3b8fd8de00c495c1cc02630a",
    )
}

fn next_local_revision_id() -> String {
    format!(
        "local-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
