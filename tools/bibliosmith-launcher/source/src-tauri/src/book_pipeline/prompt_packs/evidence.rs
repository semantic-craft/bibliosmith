//! Expert receipt and QA evidence verification. This policy is content-bound
//! to a revision and therefore survives copying a built-in into a local pack.

use super::*;

impl PromptPackStore {
    pub fn validate_expert_receipt(
        &self,
        reference: &PromptPackReference,
        receipt: &serde_json::Value,
        handoff_sha256: &str,
        project_root: &Path,
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
        let mut evidence_documents = BTreeMap::new();
        for evidence_type in &revision.required_evidence {
            let evidence = stage_evidence
                .get(evidence_type)
                .ok_or_else(|| format!("missing_translation_stage_evidence:{evidence_type}"))?;
            let document = load_stage_evidence(
                project_root,
                evidence,
                evidence_type,
                reference,
                handoff_sha256,
            )
            .map_err(|_| format!("invalid_translation_stage_evidence:{evidence_type}"))?;
            evidence_documents.insert(evidence_type.as_str(), document);
        }
        if let Some(policy) = &revision.evidence_policy {
            let independent_review = evidence_documents
                .get("independent-review")
                .ok_or_else(|| "independent_review_evidence_missing".to_string())?;
            let translator = independent_review
                .get("translatorId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let reviewer = independent_review
                .get("independentReviewerId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if policy.independent_review
                && (translator.is_empty() || reviewer.is_empty() || translator == reviewer)
            {
                return Err("independent_review_not_isolated".into());
            }
            if policy.require_zero_open_issues
                && independent_review
                    .get("latestReviewOpenIssueCount")
                    .and_then(serde_json::Value::as_u64)
                    .is_none_or(|count| count != 0)
            {
                return Err("expert_gate_open_issues".into());
            }
            if policy.require_defect_family_closure {
                let families = evidence_documents
                    .get("defect-family-closure")
                    .ok_or_else(|| "defect_family_evidence_missing".to_string())?
                    .get("defectFamilies")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| "defect_family_evidence_missing".to_string())?;
                for family in families {
                    validate_closed_defect_family(project_root, family)?;
                }
            }
        }
        Ok(())
    }
}

fn evidence_reference_path(
    project_root: &Path,
    value: &serde_json::Value,
) -> Result<PathBuf, String> {
    let path = value
        .get("path")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "evidence_path_missing".to_string())?;
    let expected_sha256 = value
        .get("sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "evidence_sha256_missing".to_string())?;
    let relative = Path::new(path);
    let mut components = relative.components();
    if !matches!(components.next(), Some(std::path::Component::Normal(root)) if root == "qa")
        || components.any(|component| !matches!(component, std::path::Component::Normal(_)))
        || expected_sha256.len() != 64
        || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("unsafe_evidence_reference".into());
    }
    let evidence_path = project_root.join(relative);
    if !evidence_path.is_file() || sha256_file(&evidence_path)? != expected_sha256 {
        return Err("evidence_content_hash_mismatch".into());
    }
    Ok(evidence_path)
}

fn load_stage_evidence(
    project_root: &Path,
    value: &serde_json::Value,
    evidence_type: &str,
    reference: &PromptPackReference,
    handoff_sha256: &str,
) -> Result<serde_json::Value, String> {
    let path = evidence_reference_path(project_root, value)?;
    let document: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
    if document.get("schema").and_then(serde_json::Value::as_str)
        != Some("translation-stage-evidence-v1")
        || document
            .get("evidenceType")
            .and_then(serde_json::Value::as_str)
            != Some(evidence_type)
        || document.get("promptPackReference") != serde_json::to_value(reference).ok().as_ref()
        || document
            .get("translationHandoffSha256")
            .and_then(serde_json::Value::as_str)
            != Some(handoff_sha256)
        || document.get("status").and_then(serde_json::Value::as_str) != Some("passed")
    {
        return Err("stage_evidence_contract_mismatch".into());
    }
    Ok(document)
}

fn validate_closed_defect_family(
    project_root: &Path,
    family: &serde_json::Value,
) -> Result<(), String> {
    let candidate_count = family
        .get("candidateCount")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "defect_family_candidate_count_missing".to_string())?;
    let repaired_count = family
        .get("repairedCount")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "defect_family_repaired_count_missing".to_string())?;
    let open_count = family
        .get("openCount")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| "defect_family_open_count_missing".to_string())?;
    if family.get("status").and_then(serde_json::Value::as_str) != Some("closed")
        || candidate_count == 0
        || repaired_count != candidate_count
        || open_count != 0
    {
        return Err("defect_family_not_closed".into());
    }
    let evidence_hashes = ["candidateScanEvidence", "repairEvidence", "recheckEvidence"]
        .iter()
        .map(|key| {
            family
                .get(*key)
                .ok_or_else(|| "defect_family_evidence_missing".to_string())
                .and_then(|value| {
                    evidence_reference_path(project_root, value)?;
                    value
                        .get("sha256")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| "evidence_sha256_missing".to_string())
                })
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    if evidence_hashes.len() != 3 {
        return Err("defect_family_recheck_evidence_not_new".into());
    }
    Ok(())
}
