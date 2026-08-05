//! Ephemeral expert-prompt compilation. Private source and glossary text may
//! enter this module, but the returned actual prompt is never a store field.

use super::*;

impl PromptPackStore {
    pub fn compile_expert_handoff(
        &self,
        reference: &PromptPackReference,
        source_sample: &str,
        glossary_entries: &[String],
    ) -> Result<ExpertPromptHandoff, String> {
        let revision = self.resolve_revision(reference, "expert-agent")?;
        let skill_dependency_versions = prompt_pack_skill_dependency_versions(&revision)?;
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
        let parameters = if revision.parameters.is_empty() {
            "（本方案没有开放参数）".into()
        } else {
            revision
                .parameters
                .iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let actual_prompt = format!(
            "# TRANSLATION PROMPT PACK\n{}\n\n# STAGES\n{}\n\n# OPEN PARAMETERS\n{}\n\n# CONTEXT POLICY\n{}\n\n# CURRENT SOURCE SAMPLE\n{}\n\n# GLOSSARY\n{}\n\n# EXECUTOR SAFETY\n占位符、标题、段落边界、术语约束和私人文本边界由 BiblioSmith 执行器拥有，任何阶段不得覆盖。",
            revision.display_name,
            stages,
            parameters,
            revision
                .context_policy
                .as_deref()
                .unwrap_or("仅使用最小必要上下文。"),
            source_sample,
            glossary,
        );
        Ok(ExpertPromptHandoff {
            prompt_pack_reference: reference.clone(),
            source_language: revision.source_language,
            target_language: revision.target_language,
            context_policy: revision.context_policy.unwrap_or_default(),
            required_skill_ids: revision.required_skill_ids,
            skill_dependency_versions,
            required_evidence: revision.required_evidence,
            excluded_responsibilities: revision.excluded_responsibilities,
            parameters: revision.parameters,
            prompt_pack_provenance: revision.source,
            stage_instructions: revision.stages,
            evidence_policy: revision.evidence_policy,
            actual_prompt,
        })
    }
}
