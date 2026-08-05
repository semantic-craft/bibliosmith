//! Translation Prompt Packs: immutable built-ins, append-only local copies, defaults,
//! and ephemeral compilation. Private source text is accepted only by compile methods
//! and never enters the store types below.

use super::*;

mod compiler;
mod evidence;

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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub parameters: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_policy: Option<ExpertEvidencePolicy>,
    pub stages: Vec<PromptStageTemplate>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExpertEvidencePolicy {
    #[serde(default)]
    pub independent_review: bool,
    #[serde(default)]
    pub require_zero_open_issues: bool,
    #[serde(default)]
    pub require_defect_family_closure: bool,
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
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
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
    pub skill_dependency_versions: BTreeMap<String, String>,
    pub required_evidence: Vec<String>,
    pub excluded_responsibilities: Vec<String>,
    pub parameters: BTreeMap<String, String>,
    pub prompt_pack_provenance: serde_json::Value,
    pub stage_instructions: Vec<PromptStageTemplate>,
    pub evidence_policy: Option<ExpertEvidencePolicy>,
    pub actual_prompt: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackStageDiff {
    pub stage_id: String,
    pub before_template: Option<String>,
    pub after_template: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackRevisionMetadata {
    pub display_name: String,
    pub executor: String,
    pub source_language: String,
    pub target_language: String,
    pub cost_hint: String,
    pub source: serde_json::Value,
    pub context_policy: Option<String>,
    pub required_skill_ids: Vec<String>,
    pub required_evidence: Vec<String>,
    pub excluded_responsibilities: Vec<String>,
    pub parameters: BTreeMap<String, String>,
    pub evidence_policy: Option<ExpertEvidencePolicy>,
}

impl From<&PromptPackRevision> for PromptPackRevisionMetadata {
    fn from(revision: &PromptPackRevision) -> Self {
        Self {
            display_name: revision.display_name.clone(),
            executor: revision.executor.clone(),
            source_language: revision.source_language.clone(),
            target_language: revision.target_language.clone(),
            cost_hint: revision.cost_hint.clone(),
            source: revision.source.clone(),
            context_policy: revision.context_policy.clone(),
            required_skill_ids: revision.required_skill_ids.clone(),
            required_evidence: revision.required_evidence.clone(),
            excluded_responsibilities: revision.excluded_responsibilities.clone(),
            parameters: revision.parameters.clone(),
            evidence_policy: revision.evidence_policy.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PromptPackRevisionDiff {
    pub before: PromptPackReference,
    pub after: PromptPackReference,
    pub before_metadata: PromptPackRevisionMetadata,
    pub after_metadata: PromptPackRevisionMetadata,
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

struct PromptPackStoreLock {
    path: PathBuf,
}

impl Drop for PromptPackStoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
        let state = self.load()?;
        resolve_revision_in_state(&state, reference, Some(executor))
    }

    pub fn copy_builtin(
        &self,
        source_reference: &PromptPackReference,
        display_name: &str,
    ) -> Result<PromptPackDefinition, String> {
        let source = resolve_builtin_revision(source_reference)?;
        let display_name = required_trimmed(display_name, "displayName")?;
        let _lock = self.acquire_lock()?;
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
        let source_metadata = revision
            .source
            .as_object_mut()
            .ok_or_else(|| "prompt_pack_source_metadata_invalid".to_string())?;
        source_metadata.insert("kind".into(), serde_json::json!("local-copy"));
        source_metadata.insert("sourcePackId".into(), serde_json::json!(source.pack_id));
        source_metadata.insert(
            "sourceRevisionId".into(),
            serde_json::json!(source.revision_id),
        );
        source_metadata.insert(
            "sourceContentSha256".into(),
            serde_json::json!(source.content_sha256),
        );
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
        let _lock = self.acquire_lock()?;
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
        validate_editable_parameters(&draft.parameters)?;
        let mut revision = current;
        revision.revision_id = next_local_revision_id();
        revision.display_name = display_name.into();
        revision.parameters = draft.parameters;
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
        let _lock = self.acquire_lock()?;
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
        let _lock = self.acquire_lock()?;
        let mut state = self.load()?;
        let revision = resolve_revision_in_state(&state, &reference, Some(executor))?;
        if revision.source_language != source_language
            || revision.target_language != target_language
        {
            return Err("prompt_pack_language_mismatch".into());
        }
        state.defaults.insert(
            default_key(executor, source_language, target_language),
            reference,
        );
        self.save(&state)
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
        let stages = stage_ids
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
            .collect();
        Ok(PromptPackRevisionDiff {
            before: before.clone(),
            after: after.clone(),
            before_metadata: PromptPackRevisionMetadata::from(&before_revision),
            after_metadata: PromptPackRevisionMetadata::from(&after_revision),
            stages,
        })
    }

    fn resolve_revision_without_executor(
        &self,
        reference: &PromptPackReference,
    ) -> Result<PromptPackRevision, String> {
        let state = self.load()?;
        resolve_revision_in_state(&state, reference, None)
    }

    fn acquire_lock(&self) -> Result<PromptPackStoreLock, String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "Prompt Pack store path has no parent".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let lock_path = self.path.with_extension("json.lock");
        for _ in 0..200 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut lock_file) => {
                    let guard = PromptPackStoreLock {
                        path: lock_path.clone(),
                    };
                    writeln!(lock_file, "{}", std::process::id()).map_err(|err| err.to_string())?;
                    return Ok(guard);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock_path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = fs::remove_file(&lock_path);
                    } else {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Prompt Pack store is busy".into())
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
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|err| err.to_string())?;
        Ok(())
    }
}

fn resolve_revision_in_state(
    state: &PromptPackStoreState,
    reference: &PromptPackReference,
    executor: Option<&str>,
) -> Result<PromptPackRevision, String> {
    let builtins = builtin_prompt_pack_catalog()?;
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
    if executor.is_some_and(|executor| revision.executor != executor) {
        return Err("prompt_pack_executor_mismatch".into());
    }
    Ok(revision.clone())
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
    validate_editable_parameters(&revision.parameters)?;
    prompt_pack_skill_dependency_versions(revision)?;
    let computed_content_sha256 = prompt_pack_content_sha256(revision)?;
    if computed_content_sha256 != revision.content_sha256 {
        return Err(format!(
            "prompt_pack_content_hash_mismatch:{}:{}:{}",
            revision.pack_id, revision.content_sha256, computed_content_sha256
        ));
    }
    Ok(())
}

fn prompt_pack_skill_dependency_versions(
    revision: &PromptPackRevision,
) -> Result<BTreeMap<String, String>, String> {
    let Some(value) = revision.source.get("skillVersions") else {
        return if revision.required_skill_ids.is_empty() {
            Ok(BTreeMap::new())
        } else {
            Err("invalid_prompt_pack_skill_versions".into())
        };
    };
    let object = value
        .as_object()
        .ok_or_else(|| "invalid_prompt_pack_skill_versions".to_string())?;
    let versions = object
        .iter()
        .map(|(skill_id, version)| {
            let version = version
                .as_str()
                .filter(|version| !skill_id.trim().is_empty() && !version.trim().is_empty())
                .ok_or_else(|| "invalid_prompt_pack_skill_versions".to_string())?;
            Ok((skill_id.clone(), version.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    if revision
        .required_skill_ids
        .iter()
        .any(|skill_id| !versions.contains_key(skill_id))
    {
        return Err("invalid_prompt_pack_skill_versions".into());
    }
    Ok(versions)
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
        || current
            .iter()
            .zip(edited)
            .any(|(locked, candidate)| locked.label != candidate.label)
        || edited
            .iter()
            .any(|stage| stage.label.trim().is_empty() || stage.template.trim().is_empty())
    {
        return Err("prompt_pack_executor_contract_is_read_only".into());
    }
    Ok(())
}

fn validate_editable_parameters(parameters: &BTreeMap<String, String>) -> Result<(), String> {
    const EDITABLE_KEYS: [&str; 2] = ["qualityFocus", "styleGuidance"];
    if parameters.iter().any(|(key, value)| {
        !EDITABLE_KEYS.contains(&key.as_str())
            || value.trim().is_empty()
            || value.chars().count() > 2_000
    }) {
        return Err("prompt_pack_parameter_not_editable".into());
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
            "13d0d89ed81c8572311c31dbb8be56c95b583a9a0a86f779ad7ae8b1ec1e5fc7",
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
