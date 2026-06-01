use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Parser, Tree};

pub const N8N_INPUT_ADAPTATION_SCHEMA_VERSION: &str = "kria.n8n.input_adaptation.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nInputCapability {
    InputReady,
    InputReceivesButIgnores,
    NoInputSurface,
    NeedsInputReview,
}

impl Default for N8nInputCapability {
    fn default() -> Self {
        Self::NeedsInputReview
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nInputSurfaceType {
    WebhookGet,
    WebhookPost,
    Form,
    Chat,
    None,
    Unknown,
}

impl Default for N8nInputSurfaceType {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nInputParameterCandidate {
    pub mapping_id: String,
    pub node_name: String,
    pub node_type: String,
    pub parameter_path: Vec<String>,
    pub parameter_label: String,
    pub suggested_field: String,
    pub suggested_expression: String,
    pub old_value_preview: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub node_family: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub operation_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub field_role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub risk_hint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub side_effect_preview: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub requires_strong_confirmation: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub adapter_confidence: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub test_value_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nCodeNodeClassification {
    InputReady,
    PartiallyInputReady,
    InputIgnored,
    PatchPreviewAvailable,
    ManualReviewRequired,
    UnsafeBlocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nCodeLiteralHint {
    pub patch_id: String,
    pub node_id: String,
    pub node_name: String,
    pub label: String,
    pub suggested_field: String,
    pub literal_type: String,
    pub old_value_preview: String,
    pub reason: String,
    #[serde(default)]
    pub start_byte: usize,
    #[serde(default)]
    pub end_byte: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nCodeNodeReport {
    pub node_id: String,
    pub node_name: String,
    pub mode: String,
    pub language: String,
    pub code_hash: String,
    pub classification: N8nCodeNodeClassification,
    pub input_references: Vec<String>,
    pub hardcoded_literals: Vec<N8nCodeLiteralHint>,
    pub output_hints: Vec<String>,
    pub unsafe_patterns: Vec<String>,
    pub patch_eligibility: String,
    pub confidence: f32,
    pub warnings: Vec<String>,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct N8nCodePatchReview {
    pub patch_id: String,
    #[serde(default = "default_true")]
    pub accepted: bool,
    #[serde(default)]
    pub field_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nCodePatchedNode {
    pub node_id: String,
    pub node_name: String,
    pub code_hash_before: String,
    pub code_hash_after: String,
    pub accepted_fields: Vec<String>,
    pub patch_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nCodePatchPlan {
    pub schema_version: String,
    pub copy_workflow_id: String,
    pub copy_display_name: String,
    pub copy_webhook_path: String,
    pub code_node_reports: Vec<N8nCodeNodeReport>,
    pub patched_nodes: Vec<N8nCodePatchedNode>,
    pub accepted_fields: Vec<String>,
    pub rejected_fields: Vec<String>,
    pub input_schema: Value,
    pub workflow_json: Value,
    pub impact_summary: String,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum N8nV5CapabilityStatus {
    FileReady,
    OutputReviewNeeded,
    CopyPossible,
    DraftOnly,
    Unsupported,
}

impl Default for N8nV5CapabilityStatus {
    fn default() -> Self {
        Self::DraftOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nBinaryInputReport {
    pub field_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub field_name: String,
    pub field_label: String,
    pub input_kind: String,
    pub required: bool,
    pub accepted_mime_types: Vec<String>,
    pub max_size_bytes: u64,
    pub destination_path: Vec<String>,
    pub safe: bool,
    pub requires_user_file: bool,
    pub warnings: Vec<String>,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nBranchReport {
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub branch_kind: String,
    pub output_count: usize,
    pub confidence: f32,
    pub warnings: Vec<String>,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nOutputNodeCandidate {
    pub node_id: String,
    pub node_name: String,
    pub node_type: String,
    pub reason: String,
    pub confidence: f32,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nOutputSelectionReport {
    pub strategy: String,
    pub confidence: f32,
    pub preferred_required: bool,
    pub candidates: Vec<N8nOutputNodeCandidate>,
    pub warnings: Vec<String>,
    pub next_action: String,
}

impl Default for N8nOutputSelectionReport {
    fn default() -> Self {
        Self {
            strategy: "needs_review".into(),
            confidence: 0.0,
            preferred_required: true,
            candidates: Vec::new(),
            warnings: Vec::new(),
            next_action: "Refresh analysis and choose which node KRIA should show as the result."
                .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct N8nBinaryInputReview {
    pub field_id: String,
    #[serde(default = "default_true")]
    pub accepted: bool,
    #[serde(default)]
    pub field_name: String,
    #[serde(default)]
    pub test_file_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nBinaryInputCopyPlan {
    pub schema_version: String,
    pub copy_workflow_id: String,
    pub copy_display_name: String,
    pub copy_webhook_path: String,
    pub binary_input_reports: Vec<N8nBinaryInputReport>,
    pub accepted_fields: Vec<String>,
    pub rejected_fields: Vec<String>,
    pub input_schema: Value,
    pub workflow_json: Value,
    pub output_selection_report: N8nOutputSelectionReport,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nInputCapabilityReport {
    pub schema_version: String,
    pub workflow_id: String,
    pub n8n_workflow_id: String,
    pub display_name: String,
    pub input_capability: N8nInputCapability,
    pub input_surface_type: N8nInputSurfaceType,
    pub used_input_fields: Vec<String>,
    pub ignored_input_surfaces: Vec<String>,
    pub hardcoded_parameter_candidates: Vec<N8nInputParameterCandidate>,
    #[serde(default)]
    pub code_node_reports: Vec<N8nCodeNodeReport>,
    #[serde(default)]
    pub binary_input_reports: Vec<N8nBinaryInputReport>,
    #[serde(default)]
    pub branch_reports: Vec<N8nBranchReport>,
    #[serde(default)]
    pub output_selection_report: N8nOutputSelectionReport,
    #[serde(default)]
    pub v5_capability_status: N8nV5CapabilityStatus,
    pub recommended_input_fields: Vec<String>,
    pub human_summary: String,
    pub technical_details: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct N8nInputAwareMappingReview {
    pub mapping_id: String,
    #[serde(default)]
    pub field_name: String,
    #[serde(default = "default_true")]
    pub accepted: bool,
    #[serde(default)]
    pub custom_expression: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct N8nInputAwareChangedParameter {
    pub node_name: String,
    pub node_type: String,
    pub parameter_path: Vec<String>,
    pub parameter_label: String,
    pub old_value_preview: String,
    pub new_expression: String,
    pub input_field: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct N8nInputAwareCopyPlan {
    pub schema_version: String,
    pub copy_workflow_id: String,
    pub copy_display_name: String,
    pub copy_webhook_path: String,
    pub changed_parameters: Vec<N8nInputAwareChangedParameter>,
    pub accepted_fields: Vec<String>,
    pub rejected_fields: Vec<String>,
    pub input_schema: Value,
    pub workflow_json: Value,
    pub warnings: Vec<String>,
    pub blockers: Vec<String>,
}

fn default_true() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn analyze_n8n_input_capability(workflow: &Value) -> N8nInputCapabilityReport {
    let nodes = workflow_nodes(workflow);
    let input_surface_type = detect_input_surface_type(&nodes);
    let display_name = workflow_name(workflow);
    let n8n_workflow_id = string_field(workflow, &["id", "workflow_id", "workflowId"])
        .unwrap_or_else(|| slugify(&display_name));
    let workflow_id = slugify(&display_name);
    let used_input_fields = detect_used_input_fields(&nodes);
    let mut warnings = Vec::new();
    let code_node_reports = analyze_code_nodes_for_surface(&nodes, &input_surface_type);
    for report in &code_node_reports {
        warnings.extend(report.warnings.clone());
    }
    let binary_input_reports = analyze_binary_input_reports(&nodes, &input_surface_type);
    for report in &binary_input_reports {
        warnings.extend(report.warnings.clone());
    }
    let branch_reports = analyze_branch_reports(workflow, &nodes);
    for report in &branch_reports {
        warnings.extend(report.warnings.clone());
    }
    let output_selection_report =
        analyze_output_selection_report(workflow, &nodes, &branch_reports);
    warnings.extend(output_selection_report.warnings.clone());
    let v5_capability_status = determine_v5_capability_status(
        &binary_input_reports,
        &branch_reports,
        &output_selection_report,
    );
    let hardcoded_parameter_candidates =
        detect_hardcoded_parameter_candidates(&nodes, &input_surface_type, &mut warnings);
    let recommended_input_fields = recommended_fields(&hardcoded_parameter_candidates);
    let ignored_input_surfaces = if !matches!(
        input_surface_type,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) && used_input_fields.is_empty()
    {
        vec![surface_label(&input_surface_type).to_string()]
    } else {
        Vec::new()
    };

    let input_capability = if matches!(
        input_surface_type,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) {
        N8nInputCapability::NoInputSurface
    } else if !used_input_fields.is_empty() {
        N8nInputCapability::InputReady
    } else if !hardcoded_parameter_candidates.is_empty() {
        N8nInputCapability::InputReceivesButIgnores
    } else if code_node_reports.iter().any(|report| {
        matches!(
            report.classification,
            N8nCodeNodeClassification::PatchPreviewAvailable
                | N8nCodeNodeClassification::InputIgnored
                | N8nCodeNodeClassification::ManualReviewRequired
                | N8nCodeNodeClassification::UnsafeBlocked
        )
    }) {
        N8nInputCapability::NeedsInputReview
    } else {
        N8nInputCapability::NeedsInputReview
    };

    let human_summary = match input_capability {
        N8nInputCapability::InputReady => {
            "This workflow already appears to use runtime input from its trigger.".to_string()
        }
        N8nInputCapability::InputReceivesButIgnores => {
            "This workflow can receive input, but KRIA found fixed node settings that do not use prompt fields yet.".to_string()
        }
        N8nInputCapability::NoInputSurface => {
            "This workflow does not expose a runtime input surface that KRIA can adapt automatically.".to_string()
        }
        N8nInputCapability::NeedsInputReview => {
            "KRIA could not confidently determine how this workflow uses input.".to_string()
        }
    };

    let mut technical_details = Vec::new();
    for candidate in &hardcoded_parameter_candidates {
        technical_details.push(format!(
            "{}: {} = {}",
            candidate.node_name, candidate.parameter_label, candidate.old_value_preview
        ));
    }
    for report in &code_node_reports {
        technical_details.push(format!(
            "{} Code node: {:?}, {} patch hint(s)",
            report.node_name,
            report.classification,
            report.hardcoded_literals.len()
        ));
    }
    for report in &binary_input_reports {
        technical_details.push(format!(
            "{} file field: {} ({})",
            report.node_name, report.field_label, report.input_kind
        ));
    }
    if output_selection_report.preferred_required {
        technical_details.push(format!(
            "Output selection needs review: {} candidate(s)",
            output_selection_report.candidates.len()
        ));
    }

    N8nInputCapabilityReport {
        schema_version: N8N_INPUT_ADAPTATION_SCHEMA_VERSION.into(),
        workflow_id,
        n8n_workflow_id,
        display_name,
        input_capability,
        input_surface_type,
        used_input_fields,
        ignored_input_surfaces,
        hardcoded_parameter_candidates,
        code_node_reports,
        binary_input_reports,
        branch_reports,
        output_selection_report,
        v5_capability_status,
        recommended_input_fields,
        human_summary,
        technical_details,
        warnings: dedupe(warnings),
    }
}

pub fn build_n8n_input_aware_copy_plan(
    workflow: &Value,
    copy_workflow_id: &str,
    copy_display_name: &str,
    mapping_reviews: &[N8nInputAwareMappingReview],
) -> N8nInputAwareCopyPlan {
    let report = analyze_n8n_input_capability(workflow);
    let warnings = report.warnings.clone();
    let mut blockers = Vec::new();

    if !matches!(
        report.input_capability,
        N8nInputCapability::InputReceivesButIgnores | N8nInputCapability::NeedsInputReview
    ) {
        blockers.push(match report.input_capability {
            N8nInputCapability::InputReady => {
                "This workflow already appears to use input; KRIA should run it with structured input instead of creating a copy.".into()
            }
            N8nInputCapability::NoInputSurface => {
                "This workflow has no compatible input surface. KRIA cannot create an input-aware copy automatically.".into()
            }
            _ => "This workflow is not eligible for an input-aware copy.".into(),
        });
    }

    if matches!(report.input_surface_type, N8nInputSurfaceType::Unknown) {
        blockers.push(
            "KRIA could not identify a supported Webhook, Form, or Chat input surface.".into(),
        );
    }

    let review_by_id = mapping_reviews
        .iter()
        .map(|review| (review.mapping_id.clone(), review.clone()))
        .collect::<BTreeMap<_, _>>();
    let candidates = if mapping_reviews.is_empty() {
        report
            .hardcoded_parameter_candidates
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        report
            .hardcoded_parameter_candidates
            .iter()
            .filter(|candidate| {
                review_by_id
                    .get(&candidate.mapping_id)
                    .map(|review| review.accepted)
                    .unwrap_or(false)
            })
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
    };

    if candidates.is_empty() && blockers.is_empty() {
        blockers.push("No safe input fields were accepted for input adaptation.".into());
    }

    let suffix = short_suffix(&format!(
        "{}:{}:{}",
        report.n8n_workflow_id, copy_workflow_id, copy_display_name
    ));
    let copy_webhook_path = format!("kria-input-{}-{}", slugify(copy_workflow_id), suffix);
    let mut copy = prepare_copy_workflow_json(workflow, copy_display_name, &copy_webhook_path);
    let mut changed_parameters = Vec::new();
    let mut accepted_fields = Vec::new();
    let mut rejected_fields = Vec::new();

    for candidate in &report.hardcoded_parameter_candidates {
        let review = review_by_id.get(&candidate.mapping_id);
        let accepted = if mapping_reviews.is_empty() {
            candidates
                .iter()
                .any(|accepted| accepted.mapping_id == candidate.mapping_id)
        } else {
            review.map(|review| review.accepted).unwrap_or(false)
        };
        if !accepted {
            rejected_fields.push(candidate.suggested_field.clone());
            continue;
        }

        let field_name = review
            .map(|review| review.field_name.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(&candidate.suggested_field)
            .to_string();
        let expression = review
            .map(|review| review.custom_expression.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                expression_for_surface(
                    &report.input_surface_type,
                    &field_name,
                    &candidate.old_value_preview,
                )
            });

        match set_node_parameter_value(
            &mut copy,
            &candidate.node_name,
            &candidate.parameter_path,
            Value::String(expression.clone()),
        ) {
            Ok(()) => {
                accepted_fields.push(field_name.clone());
                changed_parameters.push(N8nInputAwareChangedParameter {
                    node_name: candidate.node_name.clone(),
                    node_type: candidate.node_type.clone(),
                    parameter_path: candidate.parameter_path.clone(),
                    parameter_label: candidate.parameter_label.clone(),
                    old_value_preview: candidate.old_value_preview.clone(),
                    new_expression: expression,
                    input_field: field_name,
                });
            }
            Err(error) => blockers.push(error),
        }
    }

    accepted_fields = dedupe(accepted_fields);
    rejected_fields = dedupe(rejected_fields);
    let input_schema = build_input_schema(&accepted_fields);

    N8nInputAwareCopyPlan {
        schema_version: N8N_INPUT_ADAPTATION_SCHEMA_VERSION.into(),
        copy_workflow_id: copy_workflow_id.to_string(),
        copy_display_name: copy_display_name.to_string(),
        copy_webhook_path,
        changed_parameters,
        accepted_fields,
        rejected_fields,
        input_schema,
        workflow_json: copy,
        warnings: dedupe(warnings),
        blockers: dedupe(blockers),
    }
}

pub fn build_n8n_code_input_aware_copy_plan(
    workflow: &Value,
    copy_workflow_id: &str,
    copy_display_name: &str,
    patch_reviews: &[N8nCodePatchReview],
) -> N8nCodePatchPlan {
    let report = analyze_n8n_input_capability(workflow);
    let mut warnings = report.warnings.clone();
    let mut blockers = Vec::new();

    if matches!(
        report.input_surface_type,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) {
        blockers.push(
            "KRIA could not identify a supported Webhook, Form, or Chat input surface for Code patching."
                .into(),
        );
    }
    if report.code_node_reports.is_empty() {
        blockers.push("No n8n Code node was found in this workflow.".into());
    }
    if report.code_node_reports.iter().any(|node| {
        matches!(
            node.classification,
            N8nCodeNodeClassification::UnsafeBlocked
        )
    }) {
        blockers.push(
            "Unsafe Code patterns were detected. KRIA will not create an automatic patched copy."
                .into(),
        );
    }

    let review_by_id = patch_reviews
        .iter()
        .map(|review| (review.patch_id.clone(), review.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut selected = Vec::new();
    for node_report in &report.code_node_reports {
        if node_report.patch_eligibility != "auto_patch" {
            continue;
        }
        for hint in &node_report.hardcoded_literals {
            let accepted = if patch_reviews.is_empty() {
                true
            } else {
                review_by_id
                    .get(&hint.patch_id)
                    .map(|review| review.accepted)
                    .unwrap_or(false)
            };
            if accepted {
                selected.push(hint.clone());
            }
        }
    }
    selected.truncate(8);
    if selected.is_empty() && blockers.is_empty() {
        blockers.push("No safe Code literal patch was accepted.".into());
    }

    let suffix = short_suffix(&format!(
        "{}:{}:{}:code",
        report.n8n_workflow_id, copy_workflow_id, copy_display_name
    ));
    let copy_webhook_path = format!("kria-code-{}-{}", slugify(copy_workflow_id), suffix);
    let mut copy = prepare_copy_workflow_json(workflow, copy_display_name, &copy_webhook_path);
    let mut patched_nodes = Vec::new();
    let mut accepted_fields = Vec::new();
    let mut rejected_fields = Vec::new();

    if blockers.is_empty() {
        for node_report in &report.code_node_reports {
            let mut node_hints = selected
                .iter()
                .filter(|hint| hint.node_id == node_report.node_id)
                .cloned()
                .collect::<Vec<_>>();
            if node_hints.is_empty() {
                continue;
            }
            let node = workflow_nodes(workflow)
                .into_iter()
                .find(|node| code_node_id(node) == node_report.node_id);
            let Some(node) = node else {
                blockers.push(format!(
                    "Code node '{}' was not found while building the patched copy.",
                    node_report.node_name
                ));
                continue;
            };
            let Some(code) = code_text_from_node(node) else {
                blockers.push(format!(
                    "Code node '{}' has no JavaScript body.",
                    node_report.node_name
                ));
                continue;
            };
            let before_hash = sha256_hex(&code);
            if before_hash != node_report.code_hash {
                blockers.push(format!(
                    "Code node '{}' changed after analysis. Refresh before creating a copy.",
                    node_report.node_name
                ));
                continue;
            }
            node_hints.sort_by(|a, b| b.start_byte.cmp(&a.start_byte));
            let input_var = unique_code_input_var(&code);
            let mut patched = code.clone();
            let mut fields_for_node = Vec::new();
            let mut patch_ids = Vec::new();
            for hint in node_hints {
                if hint.start_byte >= hint.end_byte || hint.end_byte > code.len() {
                    blockers.push(format!(
                        "Code patch '{}' has an invalid source range.",
                        hint.patch_id
                    ));
                    continue;
                }
                let field_name = review_by_id
                    .get(&hint.patch_id)
                    .map(|review| review.field_name.trim())
                    .filter(|value| !value.is_empty())
                    .unwrap_or(&hint.suggested_field)
                    .to_string();
                let old_literal = &code[hint.start_byte..hint.end_byte];
                let expression =
                    code_input_expression(&input_var, &field_name, old_literal, &hint.literal_type);
                patched.replace_range(hint.start_byte..hint.end_byte, &expression);
                accepted_fields.push(field_name.clone());
                fields_for_node.push(field_name);
                patch_ids.push(hint.patch_id);
            }
            if !fields_for_node.is_empty() {
                patched = format!(
                    "{}\n{}",
                    code_input_prelude(&input_var, &report.input_surface_type),
                    patched
                );
                match set_code_node_text(&mut copy, &node_report.node_id, &patched) {
                    Ok(()) => patched_nodes.push(N8nCodePatchedNode {
                        node_id: node_report.node_id.clone(),
                        node_name: node_report.node_name.clone(),
                        code_hash_before: before_hash,
                        code_hash_after: sha256_hex(&patched),
                        accepted_fields: dedupe(fields_for_node),
                        patch_ids,
                    }),
                    Err(error) => blockers.push(error),
                }
            }
        }
    }

    for node_report in &report.code_node_reports {
        for hint in &node_report.hardcoded_literals {
            if !accepted_fields.contains(&hint.suggested_field) {
                rejected_fields.push(hint.suggested_field.clone());
            }
        }
    }
    accepted_fields = dedupe(accepted_fields);
    rejected_fields = dedupe(rejected_fields);
    if patched_nodes.is_empty() && blockers.is_empty() {
        blockers.push("KRIA did not produce any Code node patch.".into());
    }
    warnings = dedupe(warnings);
    let input_schema = build_input_schema(&accepted_fields);
    let impact_summary = if accepted_fields.is_empty() {
        "KRIA could not prepare an automatic Code patch. Review the blockers and update the Code node manually in n8n.".into()
    } else {
        format!(
            "KRIA will create a copied workflow that reads {} from prompt input and keeps the current fixed values as fallbacks. Original workflow is unchanged.",
            accepted_fields.join(", ")
        )
    };

    N8nCodePatchPlan {
        schema_version: N8N_INPUT_ADAPTATION_SCHEMA_VERSION.into(),
        copy_workflow_id: copy_workflow_id.to_string(),
        copy_display_name: copy_display_name.to_string(),
        copy_webhook_path,
        code_node_reports: report.code_node_reports,
        patched_nodes,
        accepted_fields,
        rejected_fields,
        input_schema,
        workflow_json: copy,
        impact_summary,
        warnings,
        blockers: dedupe(blockers),
    }
}

pub fn build_n8n_binary_input_aware_copy_plan(
    workflow: &Value,
    copy_workflow_id: &str,
    copy_display_name: &str,
    file_reviews: &[N8nBinaryInputReview],
    preferred_output_node: Option<&str>,
) -> N8nBinaryInputCopyPlan {
    let report = analyze_n8n_input_capability(workflow);
    let mut warnings = report.warnings.clone();
    let mut blockers = Vec::new();

    if report.binary_input_reports.is_empty() {
        blockers.push(
            "KRIA did not find a supported Form/Webhook multipart file input in this workflow."
                .into(),
        );
    }
    if matches!(
        report.input_surface_type,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) {
        blockers.push(
            "File-input copies need a supported Form, Webhook, or Chat/Webhook input surface."
                .into(),
        );
    }
    if report.binary_input_reports.iter().any(|item| !item.safe) {
        blockers.push(
            "One or more file fields look unsafe or unsupported for automatic copying.".into(),
        );
    }
    if report.output_selection_report.preferred_required
        && preferred_output_node
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        blockers.push("This workflow has multiple possible results. Choose a preferred output node before creating an approved copy.".into());
    }

    let review_by_id = file_reviews
        .iter()
        .map(|review| (review.field_id.clone(), review.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut accepted_reports = Vec::new();
    let mut rejected_fields = Vec::new();
    for file_report in &report.binary_input_reports {
        let review = review_by_id.get(&file_report.field_id);
        let accepted = if file_reviews.is_empty() {
            file_report.safe
        } else {
            review.map(|review| review.accepted).unwrap_or(false)
        };
        if accepted {
            accepted_reports.push(file_report.clone());
        } else {
            rejected_fields.push(file_report.field_name.clone());
        }
    }
    if accepted_reports.is_empty() && blockers.is_empty() {
        blockers.push("No file input fields were accepted for the generated copy.".into());
    }

    let suffix = short_suffix(&format!(
        "{}:{}:{}:binary",
        report.n8n_workflow_id, copy_workflow_id, copy_display_name
    ));
    let copy_webhook_path = format!("kria-file-{}-{}", slugify(copy_workflow_id), suffix);
    let copy = prepare_copy_workflow_json(workflow, copy_display_name, &copy_webhook_path);
    let mut accepted_fields = Vec::new();
    for file_report in &accepted_reports {
        let field_name = review_by_id
            .get(&file_report.field_id)
            .map(|review| review.field_name.trim())
            .filter(|value| !value.is_empty())
            .unwrap_or(&file_report.field_name)
            .to_string();
        if field_name.trim().is_empty() || is_sensitive_key(&normalize_key(&field_name)) {
            blockers.push(format!(
                "File field '{}' needs a safe prompt field name.",
                file_report.field_label
            ));
            continue;
        }
        accepted_fields.push(field_name);
    }
    accepted_fields = dedupe(accepted_fields);
    rejected_fields = dedupe(rejected_fields);
    warnings = dedupe(warnings);
    let input_schema = build_binary_input_schema(&accepted_reports, &accepted_fields);

    N8nBinaryInputCopyPlan {
        schema_version: N8N_INPUT_ADAPTATION_SCHEMA_VERSION.into(),
        copy_workflow_id: copy_workflow_id.to_string(),
        copy_display_name: copy_display_name.to_string(),
        copy_webhook_path,
        binary_input_reports: accepted_reports,
        accepted_fields,
        rejected_fields,
        input_schema,
        workflow_json: copy,
        output_selection_report: report.output_selection_report,
        warnings,
        blockers: dedupe(blockers),
    }
}

fn workflow_nodes(workflow: &Value) -> Vec<&Value> {
    workflow
        .get("nodes")
        .and_then(Value::as_array)
        .map(|nodes| nodes.iter().collect())
        .unwrap_or_default()
}

fn workflow_name(workflow: &Value) -> String {
    string_field(workflow, &["name", "display_name", "workflow_name"])
        .unwrap_or_else(|| string_field(workflow, &["id"]).unwrap_or_else(|| "n8n workflow".into()))
}

fn node_type(node: &Value) -> String {
    string_field(node, &["type"]).unwrap_or_default()
}

fn lower_node_type(node: &Value) -> String {
    node_type(node)
        .to_ascii_lowercase()
        .replace(['-', '_', ' '], "")
}

fn node_name(node: &Value) -> String {
    string_field(node, &["name"]).unwrap_or_else(|| node_type(node))
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn detect_input_surface_type(nodes: &[&Value]) -> N8nInputSurfaceType {
    for node in nodes {
        let node_type = lower_node_type(node);
        if node_type.contains("formtrigger") {
            return N8nInputSurfaceType::Form;
        }
        if node_type.contains("chattrigger") {
            return N8nInputSurfaceType::Chat;
        }
        if node_type.contains("webhook") && !node_type.contains("respondtowebhook") {
            let method = node
                .get("parameters")
                .and_then(|parameters| {
                    parameters
                        .get("httpMethod")
                        .or_else(|| parameters.get("method"))
                })
                .and_then(Value::as_str)
                .unwrap_or("POST")
                .trim()
                .to_ascii_uppercase();
            return if method == "GET" {
                N8nInputSurfaceType::WebhookGet
            } else {
                N8nInputSurfaceType::WebhookPost
            };
        }
    }
    if nodes
        .iter()
        .any(|node| lower_node_type(node).contains("trigger"))
    {
        N8nInputSurfaceType::None
    } else {
        N8nInputSurfaceType::Unknown
    }
}

fn surface_label(surface: &N8nInputSurfaceType) -> &'static str {
    match surface {
        N8nInputSurfaceType::WebhookGet => "Webhook GET query",
        N8nInputSurfaceType::WebhookPost => "Webhook POST body",
        N8nInputSurfaceType::Form => "Form submission",
        N8nInputSurfaceType::Chat => "Chat input",
        N8nInputSurfaceType::None => "No input surface",
        N8nInputSurfaceType::Unknown => "Unknown input surface",
    }
}

fn detect_used_input_fields(nodes: &[&Value]) -> Vec<String> {
    let mut fields = BTreeSet::new();
    for node in nodes.iter().filter(|node| !is_trigger_node(node)) {
        collect_input_references(node, &mut fields);
    }
    fields.into_iter().collect()
}

fn analyze_code_nodes_for_surface(
    nodes: &[&Value],
    surface: &N8nInputSurfaceType,
) -> Vec<N8nCodeNodeReport> {
    nodes
        .iter()
        .filter(|node| lower_node_type(node).contains("code"))
        .map(|node| analyze_code_node(node, surface))
        .collect()
}

fn analyze_code_node(node: &Value, surface: &N8nInputSurfaceType) -> N8nCodeNodeReport {
    let node_id = code_node_id(node);
    let node_name = node_name(node);
    let mode = code_node_mode(node);
    let language = "javascript".to_string();
    let code = code_text_from_node(node).unwrap_or_default();
    let code_hash = sha256_hex(&code);
    let input_references = code_input_references(&code);
    let output_hints = code_output_hints(&code);
    let unsafe_patterns = code_unsafe_patterns(&code);
    let mut warnings = Vec::new();

    let parse_result = parse_javascript(&code);
    let mut hardcoded_literals = Vec::new();
    let mut parse_ok = false;
    let mut complex = false;
    match parse_result {
        Ok(tree) => {
            parse_ok = true;
            complex = code_tree_has_complex_control_flow(&tree, &code);
            if !complex && unsafe_patterns.is_empty() {
                hardcoded_literals = collect_code_literal_hints(node, &tree, &code);
            }
        }
        Err(error) => warnings.push(error),
    }

    let (classification, patch_eligibility, confidence, next_action) = if !unsafe_patterns
        .is_empty()
    {
        (
            N8nCodeNodeClassification::UnsafeBlocked,
            "blocked".to_string(),
            0.2,
            "Unsafe Code patterns were detected. KRIA will not auto-patch this node; review it manually in n8n.".to_string(),
        )
    } else if !input_references.is_empty() && hardcoded_literals.is_empty() {
        (
            N8nCodeNodeClassification::InputReady,
            "not_needed".to_string(),
            0.85,
            "This Code node already appears to read runtime input.".to_string(),
        )
    } else if !input_references.is_empty() && !hardcoded_literals.is_empty() {
        (
            N8nCodeNodeClassification::PartiallyInputReady,
            "manual_suggestion".to_string(),
            0.65,
            "This Code node uses some input but still has fixed literals. Review before patching."
                .to_string(),
        )
    } else if parse_ok
        && !complex
        && !hardcoded_literals.is_empty()
        && !matches!(
            surface,
            N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
        )
    {
        (
            N8nCodeNodeClassification::PatchPreviewAvailable,
            "auto_patch".to_string(),
            0.8,
            "KRIA can prepare a copied workflow that reads these values from prompt input."
                .to_string(),
        )
    } else if !hardcoded_literals.is_empty() {
        (
            N8nCodeNodeClassification::InputIgnored,
            "manual_suggestion".to_string(),
            0.55,
            "This Code node has fixed values, but KRIA needs manual review before patching."
                .to_string(),
        )
    } else {
        (
            N8nCodeNodeClassification::ManualReviewRequired,
            "manual_suggestion".to_string(),
            0.4,
            "KRIA could not confidently patch this Code node. Review it manually in n8n."
                .to_string(),
        )
    };

    if complex {
        warnings.push("Code has control flow, functions, classes, async, or binary/file patterns that require manual review.".into());
    }
    if matches!(
        surface,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) && matches!(
        classification,
        N8nCodeNodeClassification::PatchPreviewAvailable | N8nCodeNodeClassification::InputIgnored
    ) {
        warnings.push(
            "Code patching needs a supported input surface such as Webhook, Form, or Chat.".into(),
        );
    }

    N8nCodeNodeReport {
        node_id,
        node_name,
        mode,
        language,
        code_hash,
        classification,
        input_references,
        hardcoded_literals,
        output_hints,
        unsafe_patterns,
        patch_eligibility,
        confidence,
        warnings: dedupe(warnings),
        next_action,
    }
}

fn code_node_id(node: &Value) -> String {
    string_field(node, &["id"]).unwrap_or_else(|| slugify(&node_name(node)))
}

fn code_node_mode(node: &Value) -> String {
    node.get("parameters")
        .and_then(|parameters| {
            string_field(
                parameters,
                &["mode", "jsCodeMode", "executionMode", "runMode"],
            )
        })
        .unwrap_or_else(|| "unknown".into())
}

fn code_text_from_node(node: &Value) -> Option<String> {
    node.get("parameters").and_then(|parameters| {
        string_field(
            parameters,
            &["jsCode", "code", "functionCode", "javascript", "sourceCode"],
        )
    })
}

fn set_code_node_text(workflow: &mut Value, node_id: &str, code: &str) -> Result<(), String> {
    let Some(nodes) = workflow.get_mut("nodes").and_then(Value::as_array_mut) else {
        return Err("workflow JSON has no nodes array".into());
    };
    let original_name = node_id
        .rsplit_once(':')
        .map(|(_, name)| name)
        .unwrap_or(node_id);
    for node in nodes {
        let matches_id = code_node_id(node) == node_id;
        let matches_name =
            slugify(&node_name(node)) == slugify(original_name) || node_name(node) == original_name;
        if !matches_id && !matches_name {
            continue;
        }
        let Some(parameters) = node.get_mut("parameters").and_then(Value::as_object_mut) else {
            return Err(format!("Code node '{node_id}' has no parameters object"));
        };
        let key = if parameters.contains_key("jsCode") {
            "jsCode"
        } else if parameters.contains_key("code") {
            "code"
        } else if parameters.contains_key("functionCode") {
            "functionCode"
        } else {
            "jsCode"
        };
        parameters.insert(key.into(), Value::String(code.to_string()));
        return Ok(());
    }
    Err(format!(
        "Code node '{node_id}' was not found in copied workflow JSON"
    ))
}

fn parse_javascript(code: &str) -> Result<Tree, String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::language())
        .map_err(|error| format!("JavaScript parser could not load: {error}"))?;
    let tree = parser
        .parse(code, None)
        .ok_or_else(|| "JavaScript parser returned no syntax tree".to_string())?;
    if tree.root_node().has_error() {
        return Err("Code node JavaScript has parse errors; manual review is required.".into());
    }
    Ok(tree)
}

fn code_tree_has_complex_control_flow(tree: &Tree, source: &str) -> bool {
    fn walk(node: Node<'_>, source: &str) -> bool {
        let kind = node.kind();
        if matches!(
            kind,
            "if_statement"
                | "for_statement"
                | "for_in_statement"
                | "while_statement"
                | "do_statement"
                | "switch_statement"
                | "function_declaration"
                | "function"
                | "arrow_function"
                | "class_declaration"
                | "try_statement"
                | "catch_clause"
                | "await_expression"
        ) {
            return true;
        }
        if kind == "member_expression" {
            let text = node.utf8_text(source.as_bytes()).unwrap_or_default();
            let lower = text.to_ascii_lowercase();
            if lower.contains("binary") || lower.contains("buffer") || lower.contains("file") {
                return true;
            }
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if walk(child, source) {
                return true;
            }
        }
        false
    }
    walk(tree.root_node(), source)
}

fn collect_code_literal_hints(node: &Value, tree: &Tree, source: &str) -> Vec<N8nCodeLiteralHint> {
    let mut hints = Vec::new();
    collect_code_literal_hints_from_node(node, tree.root_node(), source, &mut hints);
    hints.sort_by(|a, b| a.patch_id.cmp(&b.patch_id));
    hints.truncate(8);
    hints
}

fn collect_code_literal_hints_from_node(
    workflow_node: &Value,
    ast_node: Node<'_>,
    source: &str,
    hints: &mut Vec<N8nCodeLiteralHint>,
) {
    match ast_node.kind() {
        "variable_declarator" => {
            if let (Some(name), Some(value)) = (
                ast_node.child_by_field_name("name"),
                ast_node.child_by_field_name("value"),
            ) {
                let label = name.utf8_text(source.as_bytes()).unwrap_or("").trim();
                maybe_push_code_literal_hint(workflow_node, label, value, source, hints);
            }
        }
        "pair" => {
            if let (Some(key), Some(value)) = (
                ast_node.child_by_field_name("key"),
                ast_node.child_by_field_name("value"),
            ) {
                let label = key
                    .utf8_text(source.as_bytes())
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'');
                maybe_push_code_literal_hint(workflow_node, label, value, source, hints);
            }
        }
        _ => {}
    }
    let mut cursor = ast_node.walk();
    for child in ast_node.children(&mut cursor) {
        collect_code_literal_hints_from_node(workflow_node, child, source, hints);
    }
}

fn maybe_push_code_literal_hint(
    workflow_node: &Value,
    label: &str,
    value_node: Node<'_>,
    source: &str,
    hints: &mut Vec<N8nCodeLiteralHint>,
) {
    if label.trim().is_empty() || is_sensitive_key(label) {
        return;
    }
    let kind = value_node.kind();
    let literal_type = match kind {
        "string" | "template_string" => "string",
        "number" => "number",
        "true" | "false" => "boolean",
        _ => return,
    };
    let old_literal = value_node.utf8_text(source.as_bytes()).unwrap_or("").trim();
    let old_value_preview = literal_preview(old_literal, literal_type);
    if old_value_preview.is_empty()
        || literal_value_is_sensitive(label, &old_value_preview)
        || old_value_preview.len() > 120
    {
        return;
    }
    let node_id = code_node_id(workflow_node);
    let suggested_field = suggested_code_field(label);
    hints.push(N8nCodeLiteralHint {
        patch_id: format!("code:{}:{}:{}", node_id, normalize_key(label), value_node.start_byte()),
        node_id,
        node_name: node_name(workflow_node),
        label: label.to_string(),
        suggested_field,
        literal_type: literal_type.into(),
        old_value_preview,
        reason: "Simple hardcoded Code literal can safely fall back to its current value in a copied workflow.".into(),
        start_byte: value_node.start_byte(),
        end_byte: value_node.end_byte(),
    });
}

fn literal_preview(raw: &str, literal_type: &str) -> String {
    if literal_type == "string" {
        raw.trim()
            .trim_matches('`')
            .trim_matches('"')
            .trim_matches('\'')
            .to_string()
    } else {
        raw.trim().to_string()
    }
}

fn literal_value_is_sensitive(label: &str, value: &str) -> bool {
    let key = normalize_key(label);
    if is_sensitive_key(&key) {
        return true;
    }
    let lower = value.to_ascii_lowercase();
    lower.contains("bearer ")
        || lower.contains("api_key")
        || lower.contains("apikey")
        || lower.starts_with("sk-")
        || (value.len() >= 40
            && value
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .count()
                >= 34)
}

fn suggested_code_field(label: &str) -> String {
    let normalized = normalize_key(label);
    match normalized.as_str() {
        "t" | "title" | "movietitle" | "name" => "title".into(),
        "q" | "query" | "search" | "searchterm" => "query".into(),
        "i" | "imdbid" | "movieid" => "imdb_id".into(),
        "text" | "message" | "body" => "message".into(),
        "max" | "maxresults" | "limit" => "limit".into(),
        other if other.is_empty() => "input_value".into(),
        other => slugify(other),
    }
}

fn code_input_references(code: &str) -> Vec<String> {
    let checks = [
        ("$json", "$json"),
        ("$input", "$input"),
        ("item.json", "item.json"),
        ("items", "items"),
        ("body", "body"),
        ("query", "query"),
        ("chatInput", "chatInput"),
        ("sessionId", "sessionId"),
    ];
    checks
        .iter()
        .filter_map(|(needle, label)| code.contains(needle).then(|| (*label).to_string()))
        .collect()
}

fn code_output_hints(code: &str) -> Vec<String> {
    let checks = ["return", "json", "result", "output", "data", "items"];
    checks
        .iter()
        .filter_map(|needle| code.contains(needle).then(|| (*needle).to_string()))
        .collect()
}

fn code_unsafe_patterns(code: &str) -> Vec<String> {
    let lower = code.to_ascii_lowercase();
    let mut patterns = Vec::new();
    for (needle, label) in [
        ("eval(", "eval"),
        ("function(", "dynamic_function"),
        ("process.env", "process_env"),
        ("require(", "require"),
        ("fs.", "filesystem"),
        ("child_process", "child_process"),
        ("fetch(", "network_call"),
        ("axios.", "network_call"),
        ("http.", "network_call"),
        ("https.", "network_call"),
        ("request(", "network_call"),
        ("binary", "binary_or_file"),
    ] {
        if lower.contains(needle) {
            patterns.push(label.to_string());
        }
    }
    for line in code.lines() {
        let normalized = line.to_ascii_lowercase();
        if normalized.contains("token")
            || normalized.contains("apikey")
            || normalized.contains("api_key")
            || normalized.contains("secret")
            || normalized.contains("password")
        {
            patterns.push("secret_like_literal".into());
            break;
        }
    }
    dedupe(patterns)
}

fn unique_code_input_var(code: &str) -> String {
    if !code.contains("__kriaInput") {
        return "__kriaInput".into();
    }
    for index in 2..=20 {
        let candidate = format!("__kriaInput{index}");
        if !code.contains(&candidate) {
            return candidate;
        }
    }
    format!("__kriaInput{}", short_suffix(code))
}

fn code_input_prelude(input_var: &str, surface: &N8nInputSurfaceType) -> String {
    let source = match surface {
        N8nInputSurfaceType::WebhookGet => {
            r#"((typeof $json !== "undefined" && (($json && $json.query) || ($json && $json.body) || $json)) || (typeof $input !== "undefined" && $input.first && $input.first().json) || {})"#
        }
        N8nInputSurfaceType::Form | N8nInputSurfaceType::WebhookPost => {
            r#"((typeof $json !== "undefined" && (($json && $json.body) || ($json && $json.query) || $json)) || (typeof $input !== "undefined" && $input.first && $input.first().json) || {})"#
        }
        N8nInputSurfaceType::Chat => {
            r#"((typeof $json !== "undefined" && (($json && $json.body) || ($json && $json.query) || $json)) || (typeof $input !== "undefined" && $input.first && $input.first().json) || {})"#
        }
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown => "{}",
    };
    format!("const {input_var} = {source};")
}

fn code_input_expression(
    input_var: &str,
    field_name: &str,
    old_literal: &str,
    literal_type: &str,
) -> String {
    let access = format!(
        "{input_var}[{}]",
        serde_json::to_string(field_name).unwrap_or_else(|_| "\"input\"".into())
    );
    match literal_type {
        "number" => format!("({access} !== undefined ? Number({access}) : {old_literal})"),
        "boolean" => format!("({access} !== undefined ? {access} : {old_literal})"),
        _ => format!("({access} !== undefined ? String({access}) : {old_literal})"),
    }
}

fn is_trigger_node(node: &Value) -> bool {
    let node_type = lower_node_type(node);
    node_type.contains("trigger")
        || (node_type.contains("webhook") && !node_type.contains("respondtowebhook"))
}

fn collect_input_references(value: &Value, fields: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            if lower.contains("$json") || lower.contains("$input") || lower.contains("chatinput") {
                fields.insert("runtime_input_expression".into());
            }
            for marker in ["body.", "query.", "chatInput", "sessionId"] {
                if text.contains(marker) {
                    fields.insert(marker.trim_end_matches('.').to_string());
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_input_references(item, fields);
            }
        }
        Value::Object(map) => {
            for item in map.values() {
                collect_input_references(item, fields);
            }
        }
        _ => {}
    }
}

fn analyze_binary_input_reports(
    nodes: &[&Value],
    surface: &N8nInputSurfaceType,
) -> Vec<N8nBinaryInputReport> {
    let mut reports = Vec::new();
    for node in nodes {
        let node_type = lower_node_type(node);
        if node_type.contains("formtrigger") {
            collect_form_file_reports(node, &mut reports);
            continue;
        }
        if node_type.contains("webhook") && !node_type.contains("respondtowebhook") {
            collect_webhook_file_reports(node, surface, &mut reports);
            continue;
        }
        if node_type.contains("httprequest") {
            collect_http_multipart_reports(node, &mut reports);
        }
    }
    reports.sort_by(|a, b| a.field_id.cmp(&b.field_id));
    reports.truncate(12);
    reports
}

fn collect_form_file_reports(node: &Value, reports: &mut Vec<N8nBinaryInputReport>) {
    let Some(parameters) = node.get("parameters") else {
        return;
    };
    collect_file_like_fields(
        parameters,
        &mut Vec::new(),
        node,
        "form_file",
        "File field from n8n Form Trigger. KRIA can ask the user to select a file before testing.",
        reports,
    );
}

fn collect_webhook_file_reports(
    node: &Value,
    surface: &N8nInputSurfaceType,
    reports: &mut Vec<N8nBinaryInputReport>,
) {
    if !matches!(surface, N8nInputSurfaceType::WebhookPost) {
        return;
    }
    let Some(parameters) = node.get("parameters") else {
        return;
    };
    collect_file_like_fields(
        parameters,
        &mut Vec::new(),
        node,
        "webhook_multipart",
        "Multipart file field from n8n Webhook. KRIA can submit it only after the user chooses a file.",
        reports,
    );
}

fn collect_http_multipart_reports(node: &Value, reports: &mut Vec<N8nBinaryInputReport>) {
    let Some(parameters) = node.get("parameters") else {
        return;
    };
    let parameter_text = parameters.to_string().to_ascii_lowercase();
    if !contains_any(
        &parameter_text,
        &[
            "multipart",
            "form-data",
            "sendbinarydata",
            "binaryproperty",
            "file",
        ],
    ) {
        return;
    }
    collect_file_like_fields(
        parameters,
        &mut Vec::new(),
        node,
        "http_multipart_passthrough",
        "HTTP Request multipart destination. KRIA can pass the selected file through the copied workflow when the destination field is clear.",
        reports,
    );
}

fn collect_file_like_fields(
    value: &Value,
    path: &mut Vec<String>,
    node: &Value,
    input_kind: &str,
    next_action: &str,
    reports: &mut Vec<N8nBinaryInputReport>,
) {
    match value {
        Value::Object(map) => {
            if let Some(report) =
                binary_report_from_object(map, path, node, input_kind, next_action)
            {
                reports.push(report);
            }
            for (key, child) in map {
                path.push(key.clone());
                collect_file_like_fields(child, path, node, input_kind, next_action, reports);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_file_like_fields(item, path, node, input_kind, next_action, reports);
                path.pop();
            }
        }
        _ => {}
    }
}

fn binary_report_from_object(
    map: &Map<String, Value>,
    path: &[String],
    node: &Value,
    input_kind: &str,
    next_action: &str,
) -> Option<N8nBinaryInputReport> {
    let label = map
        .get("fieldLabel")
        .or_else(|| map.get("label"))
        .or_else(|| map.get("name"))
        .or_else(|| map.get("fieldName"))
        .or_else(|| map.get("key"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let type_text = map
        .get("fieldType")
        .or_else(|| map.get("type"))
        .or_else(|| map.get("inputType"))
        .or_else(|| map.get("parameterType"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let joined = format!("{} {} {}", label, type_text, path.join(".")).to_ascii_lowercase();
    if !contains_any(
        &joined,
        &[
            "file",
            "upload",
            "attachment",
            "document",
            "binary",
            "image",
        ],
    ) {
        return None;
    }
    let field_name = form_trigger_submission_field_name(input_kind, path)
        .unwrap_or_else(|| suggested_file_field_name(label));
    let normalized = normalize_key(&field_name);
    let sensitive = is_sensitive_key(&normalized) || is_sensitive_key(&normalize_key(&joined));
    let accepted_mime_types = accepted_mime_types_from_object(map);
    let required = map
        .get("required")
        .or_else(|| map.get("isRequired"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let mut warnings = Vec::new();
    if sensitive {
        warnings.push(
            "File-like field is sensitive-looking and will not be adapted automatically.".into(),
        );
    }
    Some(N8nBinaryInputReport {
        field_id: slugify(&format!(
            "{}:{}:{}",
            node_name(node),
            path.join("."),
            field_name
        )),
        node_id: node
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| slugify(&node_name(node))),
        node_name: node_name(node),
        node_type: node_type(node),
        field_name,
        field_label: label.to_string(),
        input_kind: input_kind.into(),
        required,
        accepted_mime_types,
        max_size_bytes: 10 * 1024 * 1024,
        destination_path: path.to_vec(),
        safe: !sensitive,
        requires_user_file: true,
        warnings,
        next_action: next_action.into(),
    })
}

fn form_trigger_submission_field_name(input_kind: &str, path: &[String]) -> Option<String> {
    if input_kind != "form_file" {
        return None;
    }
    path.iter()
        .rev()
        .find_map(|segment| segment.parse::<usize>().ok())
        .map(|index| format!("field-{index}"))
}

fn accepted_mime_types_from_object(map: &Map<String, Value>) -> Vec<String> {
    let mut values = Vec::new();
    for key in ["accept", "acceptedTypes", "mimeType", "mimeTypes"] {
        if let Some(value) = map.get(key) {
            match value {
                Value::String(text) => {
                    values.extend(
                        text.split(',')
                            .map(str::trim)
                            .filter(|item| !item.is_empty())
                            .map(str::to_string),
                    );
                }
                Value::Array(items) => {
                    values.extend(
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::trim)
                            .filter(|item| !item.is_empty())
                            .map(str::to_string),
                    );
                }
                _ => {}
            }
        }
    }
    dedupe(values)
}

fn suggested_file_field_name(label: &str) -> String {
    let normalized = normalize_key(label);
    match normalized.as_str() {
        "file" | "upload" | "document" | "attachment" => "file".into(),
        "image" | "photo" | "picture" => "image".into(),
        other if other.is_empty() => "file".into(),
        other => slugify(other),
    }
}

fn analyze_branch_reports(workflow: &Value, nodes: &[&Value]) -> Vec<N8nBranchReport> {
    let mut reports = Vec::new();
    for node in nodes {
        let normalized_node_type = lower_node_type(node);
        let branch_kind = if normalized_node_type.contains("switch") {
            Some("switch")
        } else if normalized_node_type.contains("if") {
            Some("if")
        } else if normalized_node_type.contains("merge") {
            Some("merge")
        } else if normalized_node_type.contains("splitinbatches")
            || normalized_node_type.contains("loop")
        {
            Some("loop")
        } else {
            None
        };
        let Some(branch_kind) = branch_kind else {
            continue;
        };
        let output_count =
            connection_output_count(workflow, &node_name(node)).max(match branch_kind {
                "if" => 2,
                "switch" => 2,
                _ => 1,
            });
        reports.push(N8nBranchReport {
            node_id: node
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| slugify(&node_name(node))),
            node_name: node_name(node),
            node_type: node_type(node),
            branch_kind: branch_kind.into(),
            output_count,
            confidence: 0.85,
            warnings: vec!["Workflow has branching. KRIA will not rewrite branches in V5; it will ask which result node to show when needed.".into()],
            next_action: "Choose the preferred result node if KRIA cannot confidently pick one.".into(),
        });
    }
    reports
}

fn analyze_output_selection_report(
    workflow: &Value,
    nodes: &[&Value],
    branch_reports: &[N8nBranchReport],
) -> N8nOutputSelectionReport {
    let terminal_names = terminal_node_names(workflow, nodes);
    let mut candidates = Vec::new();
    for node in nodes.iter().filter(|node| !is_trigger_node(node)) {
        let name = node_name(node);
        let normalized_node_type = lower_node_type(node);
        let terminal = terminal_names.contains(&name);
        let response_like = contains_any(
            &normalized_node_type,
            &[
                "respondtowebhook",
                "httprequest",
                "gmail",
                "googlesheets",
                "slack",
                "postgres",
                "mysql",
                "sqlite",
                "mssql",
                "supabase",
                "code",
            ],
        );
        if !terminal && !response_like {
            continue;
        }
        let reason = if normalized_node_type.contains("respondtowebhook") {
            "Responds directly to the caller"
        } else if response_like && terminal {
            "Likely final useful output"
        } else if terminal {
            "Terminal workflow node"
        } else {
            "Response-like node"
        };
        let confidence = if normalized_node_type.contains("respondtowebhook") {
            0.95
        } else if terminal && response_like {
            0.82
        } else if terminal {
            0.68
        } else {
            0.55
        };
        candidates.push(N8nOutputNodeCandidate {
            node_id: node
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| slugify(&name)),
            node_name: name,
            node_type: node_type(node),
            reason: reason.into(),
            confidence,
            terminal,
        });
    }
    candidates.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node_name.cmp(&b.node_name))
    });
    candidates.truncate(8);

    let preferred_required = !branch_reports.is_empty() && candidates.len() > 1;
    let mut warnings = Vec::new();
    if preferred_required {
        warnings.push("Multiple possible result nodes were detected. Choose the one KRIA should show in chat and Run History.".into());
    }
    let (strategy, confidence, next_action) = match candidates.first() {
        Some(candidate) if !preferred_required && candidate.confidence >= 0.8 => (
            "auto_selected".to_string(),
            candidate.confidence,
            format!("KRIA can show output from '{}'.", candidate.node_name),
        ),
        Some(_) => (
            "preferred_output_node_required".to_string(),
            0.55,
            "Choose which node output KRIA should show before approval.".into(),
        ),
        None => (
            "execution_summary_fallback".to_string(),
            0.35,
            "KRIA could not find a clear output node. Test output will use a compact execution summary unless you choose a node later.".into(),
        ),
    };

    N8nOutputSelectionReport {
        strategy,
        confidence,
        preferred_required: preferred_required || confidence < 0.6,
        candidates,
        warnings,
        next_action,
    }
}

fn determine_v5_capability_status(
    binary_reports: &[N8nBinaryInputReport],
    branch_reports: &[N8nBranchReport],
    output_report: &N8nOutputSelectionReport,
) -> N8nV5CapabilityStatus {
    if binary_reports.iter().any(|report| !report.safe) {
        return N8nV5CapabilityStatus::Unsupported;
    }
    if !binary_reports.is_empty() && output_report.preferred_required {
        return N8nV5CapabilityStatus::OutputReviewNeeded;
    }
    if !binary_reports.is_empty() {
        return N8nV5CapabilityStatus::FileReady;
    }
    if !branch_reports.is_empty() && output_report.preferred_required {
        return N8nV5CapabilityStatus::OutputReviewNeeded;
    }
    if !branch_reports.is_empty() {
        return N8nV5CapabilityStatus::CopyPossible;
    }
    N8nV5CapabilityStatus::DraftOnly
}

fn connection_output_count(workflow: &Value, node_name: &str) -> usize {
    workflow
        .get("connections")
        .and_then(|connections| connections.get(node_name))
        .and_then(|entry| entry.get("main"))
        .and_then(Value::as_array)
        .map(|main| main.len())
        .unwrap_or(0)
}

fn terminal_node_names(workflow: &Value, nodes: &[&Value]) -> BTreeSet<String> {
    let mut outgoing = BTreeSet::new();
    if let Some(connections) = workflow.get("connections").and_then(Value::as_object) {
        for (source, entry) in connections {
            let mut targets = BTreeSet::new();
            collect_connection_targets(entry, &mut targets);
            if !targets.is_empty() {
                outgoing.insert(source.clone());
            }
        }
    }
    nodes
        .iter()
        .filter_map(|node| {
            let name = node_name(node);
            (!is_trigger_node(node) && !outgoing.contains(&name)).then_some(name)
        })
        .collect()
}

fn collect_connection_targets(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(node) = map.get("node").and_then(Value::as_str) {
                out.insert(node.to_string());
            }
            for child in map.values() {
                collect_connection_targets(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_connection_targets(item, out);
            }
        }
        _ => {}
    }
}

fn detect_hardcoded_parameter_candidates(
    nodes: &[&Value],
    surface: &N8nInputSurfaceType,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let mut candidates = Vec::new();
    if matches!(
        surface,
        N8nInputSurfaceType::None | N8nInputSurfaceType::Unknown
    ) {
        return candidates;
    }

    for node in nodes.iter().filter(|node| !is_trigger_node(node)) {
        let node_type = lower_node_type(node);
        let mut app_candidates = detect_app_node_candidates(node, warnings);
        if !app_candidates.is_empty() {
            candidates.append(&mut app_candidates);
            continue;
        }
        let supports_v1 = node_type.contains("httprequest")
            || node_type.ends_with("set")
            || node_type.contains("setfields")
            || node_type.contains("editfields");
        if !supports_v1 {
            if node_type.contains("code") {
                warnings.push(format!(
                    "{} is a Code node. KRIA can detect input references there, but V1 will not rewrite code automatically.",
                    node_name(node)
                ));
            }
            continue;
        }
        let Some(parameters) = node.get("parameters") else {
            continue;
        };
        let mut node_candidates = Vec::new();
        collect_candidates_from_parameters(
            parameters,
            &mut Vec::new(),
            node,
            &mut node_candidates,
            warnings,
        );
        candidates.extend(node_candidates);
    }

    candidates.sort_by(|a, b| {
        candidate_priority(a)
            .cmp(&candidate_priority(b))
            .then_with(|| a.mapping_id.cmp(&b.mapping_id))
    });
    candidates.truncate(12);
    candidates
}

fn candidate_priority(candidate: &N8nInputParameterCandidate) -> u8 {
    let label = candidate.parameter_label.to_ascii_lowercase();
    if candidate.requires_strong_confirmation {
        return 3;
    }
    if matches!(
        label.as_str(),
        "title" | "t" | "query" | "q" | "search" | "prompt" | "text" | "genre"
    ) {
        0
    } else if matches!(label.as_str(), "type" | "limit" | "year" | "page") {
        1
    } else {
        2
    }
}

fn detect_app_node_candidates(
    node: &Value,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let node_type = lower_node_type(node);
    if node_type.contains("gmail") {
        return detect_gmail_candidates(node, warnings);
    }
    if node_type.contains("googlesheets") || node_type.contains("sheets") {
        return detect_sheets_candidates(node, warnings);
    }
    if node_type.contains("slack") {
        return detect_slack_candidates(node, warnings);
    }
    if is_database_node_type(&node_type) {
        return detect_database_candidates(node, warnings);
    }
    Vec::new()
}

fn is_database_node_type(node_type: &str) -> bool {
    [
        "postgres",
        "postgresql",
        "mysql",
        "mariadb",
        "sqlite",
        "mssql",
        "microsoftsql",
        "supabase",
        "database",
    ]
    .iter()
    .any(|term| node_type.contains(term))
}

fn detect_gmail_candidates(
    node: &Value,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let operation = operation_signature(node);
    if contains_any(
        &operation,
        &[
            "send",
            "delete",
            "remove",
            "archive",
            "trash",
            "draft",
            "reply",
            "label",
            "modify",
            "markread",
            "markunread",
        ],
    ) {
        warnings.push(format!(
            "{} uses a Gmail write/update operation. KRIA V2 only adapts Gmail read/search/list/get operations.",
            node_name(node)
        ));
        return Vec::new();
    }
    if !contains_any(
        &operation,
        &["get", "getall", "list", "read", "search", "message"],
    ) {
        warnings.push(format!(
            "{} is a Gmail node, but KRIA could not verify a read/search operation.",
            node_name(node)
        ));
        return Vec::new();
    }
    collect_direct_app_candidates(
        node,
        "gmail",
        "read",
        "green",
        "",
        false,
        &[
            "q",
            "query",
            "search",
            "searchquery",
            "label",
            "labelid",
            "labelids",
            "from",
            "sender",
            "subject",
            "maxresults",
            "limit",
            "messageid",
            "message_id",
            "threadid",
            "thread_id",
        ],
        &["spreadsheetid", "documentid", "attachment", "raw", "body"],
        warnings,
    )
}

fn detect_sheets_candidates(
    node: &Value,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let operation = operation_signature(node);
    if contains_any(
        &operation,
        &[
            "append", "update", "delete", "clear", "create", "write", "insert", "remove",
        ],
    ) {
        warnings.push(format!(
            "{} uses a Google Sheets write/update operation. KRIA V2 only adapts read/lookup operations.",
            node_name(node)
        ));
        return Vec::new();
    }
    if !contains_any(
        &operation,
        &["get", "read", "lookup", "search", "list", "row"],
    ) {
        warnings.push(format!(
            "{} is a Google Sheets node, but KRIA could not verify a read/lookup operation.",
            node_name(node)
        ));
        return Vec::new();
    }
    collect_direct_app_candidates(
        node,
        "google_sheets",
        "read",
        "green",
        "",
        false,
        &[
            "range",
            "sheet",
            "sheetname",
            "worksheet",
            "lookup",
            "lookupvalue",
            "lookupcolumn",
            "filter",
            "query",
            "search",
            "limit",
            "maxresults",
        ],
        &["spreadsheetid", "documentid", "credential", "owner"],
        warnings,
    )
}

fn detect_slack_candidates(
    node: &Value,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let operation = operation_signature(node);
    if contains_any(
        &operation,
        &[
            "delete",
            "update",
            "invite",
            "kick",
            "archive",
            "unarchive",
            "admin",
            "user",
        ],
    ) {
        warnings.push(format!(
            "{} uses a Slack admin/update/delete operation. KRIA V2 only adapts Slack post/send message operations.",
            node_name(node)
        ));
        return Vec::new();
    }
    if !contains_any(&operation, &["post", "send", "message", "chat"]) {
        warnings.push(format!(
            "{} is a Slack node, but KRIA could not verify a post/send message operation.",
            node_name(node)
        ));
        return Vec::new();
    }
    let candidates = collect_direct_app_candidates(
        node,
        "slack",
        "post_message",
        "yellow",
        "This will post a Slack message if you test or approve the generated copy.",
        true,
        &["channel", "channelid", "room", "text", "message", "body"],
        &[
            "token",
            "auth",
            "webhook",
            "workspace",
            "team",
            "userid",
            "threadts",
        ],
        warnings,
    );
    if !candidates.is_empty() {
        warnings.push(format!(
            "{} is a Slack post workflow. KRIA will keep generated copies review-gated because posting is a side effect.",
            node_name(node)
        ));
    }
    candidates
}

fn detect_database_candidates(
    node: &Value,
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let operation = operation_signature(node);
    if contains_any(
        &operation,
        &[
            "insert",
            "update",
            "delete",
            "upsert",
            "drop",
            "truncate",
            "alter",
            "create",
            "grant",
            "revoke",
            "copy",
            "procedure",
        ],
    ) {
        warnings.push(format!(
            "{} uses a database write/admin operation. KRIA V3 only adapts read/select database operations.",
            node_name(node)
        ));
        return Vec::new();
    }

    let sql_values = collect_database_sql_values(node);
    let mut saw_safe_sql = false;
    for sql in &sql_values {
        match sql_read_safety(sql) {
            SqlReadSafety::ReadOnly => saw_safe_sql = true,
            SqlReadSafety::Unsafe(reason) => {
                warnings.push(format!(
                    "{} SQL is not safe for automatic adaptation: {reason}.",
                    node_name(node)
                ));
                return Vec::new();
            }
            SqlReadSafety::NeedsReview(reason) => {
                warnings.push(format!(
                    "{} SQL needs review before KRIA can adapt it: {reason}.",
                    node_name(node)
                ));
                return Vec::new();
            }
        }
    }

    let read_operation = contains_any(
        &operation,
        &[
            "select", "find", "get", "getall", "search", "read", "lookup", "row", "rows", "list",
        ],
    );
    if !read_operation && !saw_safe_sql {
        warnings.push(format!(
            "{} is a database node, but KRIA could not verify a read/select operation.",
            node_name(node)
        ));
        return Vec::new();
    }

    collect_direct_app_candidates(
        node,
        "database",
        "read",
        "green",
        "",
        false,
        &[
            "where",
            "filter",
            "lookup",
            "lookupvalue",
            "search",
            "searchvalue",
            "value",
            "table",
            "tablename",
            "collection",
            "limit",
            "offset",
            "from",
            "to",
            "date",
            "startdate",
            "enddate",
        ],
        &[
            "host",
            "port",
            "database",
            "dbname",
            "user",
            "username",
            "password",
            "ssl",
            "connection",
            "credential",
            "schemaowner",
            "owner",
            "role",
            "admin",
            "token",
            "secret",
            "auth",
        ],
        warnings,
    )
}

fn collect_direct_app_candidates(
    node: &Value,
    node_family: &str,
    operation_kind: &str,
    risk_hint: &str,
    side_effect_preview: &str,
    requires_strong_confirmation: bool,
    allowlist: &[&str],
    denylist: &[&str],
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let mut out = Vec::new();
    let Some(parameters) = node.get("parameters") else {
        return out;
    };
    collect_app_candidates_from_value(
        parameters,
        &mut Vec::new(),
        node,
        node_family,
        operation_kind,
        risk_hint,
        side_effect_preview,
        requires_strong_confirmation,
        allowlist,
        denylist,
        &mut out,
        warnings,
    );
    out.sort_by(|a, b| {
        candidate_priority(a)
            .cmp(&candidate_priority(b))
            .then_with(|| a.mapping_id.cmp(&b.mapping_id))
    });
    out.truncate(8);
    out
}

#[allow(clippy::too_many_arguments)]
fn collect_app_candidates_from_value(
    value: &Value,
    path: &mut Vec<String>,
    node: &Value,
    node_family: &str,
    operation_kind: &str,
    risk_hint: &str,
    side_effect_preview: &str,
    requires_strong_confirmation: bool,
    allowlist: &[&str],
    denylist: &[&str],
    out: &mut Vec<N8nInputParameterCandidate>,
    warnings: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            out.extend(direct_app_candidates_from_object(
                map,
                path,
                node,
                node_family,
                operation_kind,
                risk_hint,
                side_effect_preview,
                requires_strong_confirmation,
                allowlist,
                denylist,
                warnings,
            ));
            for (key, child) in map {
                path.push(key.clone());
                if is_app_path_allowed(key, path, allowlist, denylist) {
                    collect_app_candidates_from_value(
                        child,
                        path,
                        node,
                        node_family,
                        operation_kind,
                        risk_hint,
                        side_effect_preview,
                        requires_strong_confirmation,
                        allowlist,
                        denylist,
                        out,
                        warnings,
                    );
                }
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_app_candidates_from_value(
                    item,
                    path,
                    node,
                    node_family,
                    operation_kind,
                    risk_hint,
                    side_effect_preview,
                    requires_strong_confirmation,
                    allowlist,
                    denylist,
                    out,
                    warnings,
                );
                path.pop();
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn direct_app_candidates_from_object(
    map: &Map<String, Value>,
    path: &[String],
    node: &Value,
    node_family: &str,
    operation_kind: &str,
    risk_hint: &str,
    side_effect_preview: &str,
    requires_strong_confirmation: bool,
    allowlist: &[&str],
    denylist: &[&str],
    warnings: &mut Vec<String>,
) -> Vec<N8nInputParameterCandidate> {
    let mut out = Vec::new();
    if let Some(candidate) = candidate_from_object(map, path, node, warnings) {
        let key = normalize_key(&candidate.parameter_label);
        if !allowlist.contains(&key.as_str())
            || denylist.iter().any(|denied| key.contains(denied))
            || is_sensitive_key(&key)
        {
            return out;
        }
        out.push(with_app_metadata(
            candidate,
            node_family,
            operation_kind,
            app_field_role(node_family, &key),
            risk_hint,
            side_effect_preview,
            requires_strong_confirmation,
        ));
        return out;
    }

    for (key, value) in map {
        let normalized = normalize_key(key);
        if !allowlist.contains(&normalized.as_str())
            || denylist.iter().any(|denied| normalized.contains(denied))
            || is_sensitive_key(&normalized)
            || !is_safe_static_value(value)
        {
            continue;
        }
        let mut value_path = path.to_vec();
        value_path.push(key.clone());
        let full_text = format!("{} {}", key, value_path.join(".")).to_ascii_lowercase();
        if is_sensitive_key(&full_text) {
            warnings.push(format!(
                "Skipped sensitive-looking field '{}' in {}.",
                key,
                node_name(node)
            ));
            continue;
        }
        let suggested_field = suggested_app_field_name(node_family, key);
        let mapping_id = slugify(&format!(
            "{}:{}:{}",
            node_name(node),
            value_path.join("."),
            suggested_field
        ));
        out.push(N8nInputParameterCandidate {
            mapping_id,
            node_name: node_name(node),
            node_type: node_type(node),
            parameter_path: value_path,
            parameter_label: key.to_string(),
            suggested_expression: String::new(),
            suggested_field: suggested_field.clone(),
            old_value_preview: scalar_preview(value),
            reason: app_candidate_reason(node_family, operation_kind, risk_hint).into(),
            node_family: node_family.into(),
            operation_kind: operation_kind.into(),
            field_role: app_field_role(node_family, &normalized).into(),
            risk_hint: risk_hint.into(),
            side_effect_preview: side_effect_preview.into(),
            requires_strong_confirmation,
            adapter_confidence: "high".into(),
            test_value_hint: app_test_value_hint(node_family, &suggested_field).into(),
        });
    }
    out
}

fn with_app_metadata(
    mut candidate: N8nInputParameterCandidate,
    node_family: &str,
    operation_kind: &str,
    field_role: &str,
    risk_hint: &str,
    side_effect_preview: &str,
    requires_strong_confirmation: bool,
) -> N8nInputParameterCandidate {
    candidate.node_family = node_family.into();
    candidate.operation_kind = operation_kind.into();
    candidate.field_role = field_role.into();
    candidate.risk_hint = risk_hint.into();
    candidate.side_effect_preview = side_effect_preview.into();
    candidate.requires_strong_confirmation = requires_strong_confirmation;
    candidate.adapter_confidence = "high".into();
    candidate.test_value_hint = app_test_value_hint(node_family, &candidate.suggested_field).into();
    candidate.reason = app_candidate_reason(node_family, operation_kind, risk_hint).into();
    candidate
}

fn collect_candidates_from_parameters(
    value: &Value,
    path: &mut Vec<String>,
    node: &Value,
    out: &mut Vec<N8nInputParameterCandidate>,
    warnings: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            if let Some(candidate) = candidate_from_object(map, path, node, warnings) {
                out.push(candidate);
            }
            for (key, child) in map {
                path.push(key.clone());
                collect_candidates_from_parameters(child, path, node, out, warnings);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_candidates_from_parameters(item, path, node, out, warnings);
                path.pop();
            }
        }
        _ => {}
    }
}

fn candidate_from_object(
    map: &Map<String, Value>,
    path: &[String],
    node: &Value,
    warnings: &mut Vec<String>,
) -> Option<N8nInputParameterCandidate> {
    if !path_is_v1_editable(path) {
        return None;
    }
    let label = map
        .get("name")
        .or_else(|| map.get("fieldName"))
        .or_else(|| map.get("field"))
        .or_else(|| map.get("key"))
        .or_else(|| map.get("parameter"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let value = map
        .get("value")
        .or_else(|| map.get("fieldValue"))
        .or_else(|| map.get("defaultValue"))?;
    if !is_safe_static_value(value) {
        return None;
    }
    let mut value_path = path.to_vec();
    let value_key = if map.get("value").is_some() {
        "value"
    } else if map.get("fieldValue").is_some() {
        "fieldValue"
    } else {
        "defaultValue"
    };
    value_path.push(value_key.into());

    let full_text = format!("{} {}", label, value_path.join(".")).to_ascii_lowercase();
    if is_sensitive_key(&full_text) {
        warnings.push(format!(
            "Skipped sensitive-looking field '{}' in {}.",
            label,
            node_name(node)
        ));
        return None;
    }

    let suggested_field = suggested_field_name(label);
    let old_value_preview = scalar_preview(value);
    let mapping_id = slugify(&format!(
        "{}:{}:{}",
        node_name(node),
        value_path.join("."),
        suggested_field
    ));
    Some(N8nInputParameterCandidate {
        mapping_id,
        node_name: node_name(node),
        node_type: node_type(node),
        parameter_path: value_path,
        parameter_label: label.to_string(),
        suggested_expression: String::new(),
        suggested_field,
        old_value_preview,
        reason: "Static HTTP/Set parameter can safely fall back to its current value.".into(),
        node_family: "http_set".into(),
        operation_kind: "static_parameter".into(),
        field_role: "request_parameter".into(),
        risk_hint: "green".into(),
        side_effect_preview: String::new(),
        requires_strong_confirmation: false,
        adapter_confidence: "high".into(),
        test_value_hint: String::new(),
    })
}

fn operation_signature(node: &Value) -> String {
    let mut parts = vec![node_type(node), node_name(node)];
    if let Some(parameters) = node.get("parameters").and_then(Value::as_object) {
        for key in ["resource", "operation", "mode", "action", "type"] {
            if let Some(value) = parameters.get(key).and_then(Value::as_str) {
                parts.push(value.to_string());
            }
        }
    }
    normalize_key(&parts.join(" "))
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SqlReadSafety {
    ReadOnly,
    NeedsReview(String),
    Unsafe(String),
}

fn collect_database_sql_values(node: &Value) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(parameters) = node.get("parameters") {
        collect_database_sql_values_from_value(parameters, &mut Vec::new(), &mut values);
    }
    values
}

fn collect_database_sql_values_from_value(
    value: &Value,
    path: &mut Vec<String>,
    out: &mut Vec<String>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                path.push(key.clone());
                let normalized = normalize_key(key);
                if matches!(
                    normalized.as_str(),
                    "query" | "sql" | "statement" | "rawquery" | "sqlquery"
                ) {
                    if let Some(text) = child.as_str() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() && !trimmed.contains("={{") {
                            out.push(trimmed.to_string());
                        }
                    }
                }
                collect_database_sql_values_from_value(child, path, out);
                path.pop();
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_database_sql_values_from_value(item, path, out);
                path.pop();
            }
        }
        _ => {}
    }
}

fn sql_read_safety(sql: &str) -> SqlReadSafety {
    let trimmed = sql.trim();
    if trimmed.is_empty() {
        return SqlReadSafety::NeedsReview("SQL is empty".into());
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("--") || lower.contains("/*") || lower.contains("*/") {
        return SqlReadSafety::Unsafe("SQL contains comments".into());
    }
    if lower.contains(';') {
        return SqlReadSafety::Unsafe("SQL contains multiple or terminated statements".into());
    }
    let normalized = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    if contains_any_sql_word(
        &normalized,
        &[
            "insert", "update", "delete", "upsert", "merge", "drop", "truncate", "alter", "create",
            "grant", "revoke", "copy", "call", "execute", "exec", "replace",
        ],
    ) {
        return SqlReadSafety::Unsafe("SQL contains write/admin keywords".into());
    }
    if starts_with_sql_word(
        &normalized,
        &["select", "show", "describe", "desc", "explain"],
    ) {
        return SqlReadSafety::ReadOnly;
    }
    if normalized.starts_with("with ") && normalized.contains(" select ") {
        return SqlReadSafety::ReadOnly;
    }
    SqlReadSafety::NeedsReview("SQL does not start with a recognized read-only keyword".into())
}

fn contains_any_sql_word(text: &str, words: &[&str]) -> bool {
    words.iter().any(|word| contains_sql_word(text, word))
}

fn starts_with_sql_word(text: &str, words: &[&str]) -> bool {
    words
        .iter()
        .any(|word| text == *word || text.starts_with(&format!("{word} ")))
}

fn contains_sql_word(text: &str, word: &str) -> bool {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|part| part == word)
}

fn is_app_path_allowed(key: &str, path: &[String], _allowlist: &[&str], denylist: &[&str]) -> bool {
    let joined = normalize_key(&format!("{} {}", key, path.join(" ")));
    !denylist.iter().any(|denied| joined.contains(denied)) && !is_sensitive_key(&joined)
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn suggested_app_field_name(node_family: &str, label: &str) -> String {
    let normalized = normalize_key(label);
    match (node_family, normalized.as_str()) {
        ("gmail", "q" | "query" | "search" | "searchquery") => "email_query".into(),
        ("gmail", "from" | "sender") => "email_from".into(),
        ("gmail", "subject") => "email_subject".into(),
        ("gmail", "label" | "labelid" | "labelids") => "email_label".into(),
        ("gmail", "messageid") => "message_id".into(),
        ("gmail", "threadid") => "thread_id".into(),
        ("google_sheets", "range") => "sheet_range".into(),
        ("google_sheets", "sheet" | "sheetname" | "worksheet") => "sheet_name".into(),
        ("google_sheets", "lookup" | "lookupvalue" | "search" | "query") => "lookup_value".into(),
        ("google_sheets", "lookupcolumn" | "filter") => "lookup_column".into(),
        ("database", "query" | "sql" | "statement") => "database_query".into(),
        (
            "database",
            "where" | "filter" | "lookup" | "lookupvalue" | "search" | "searchvalue" | "value",
        ) => "lookup_value".into(),
        ("database", "table" | "tablename" | "collection") => "table_name".into(),
        ("database", "offset") => "offset".into(),
        ("database", "from" | "startdate") => "start_date".into(),
        ("database", "to" | "enddate") => "end_date".into(),
        ("slack", "channel" | "channelid" | "room") => "slack_channel".into(),
        ("slack", "text" | "message" | "body") => "slack_message".into(),
        (_, "maxresults" | "limit") => "limit".into(),
        _ => suggested_field_name(label),
    }
}

fn app_field_role(node_family: &str, normalized_key: &str) -> &'static str {
    match (node_family, normalized_key) {
        ("gmail", "q" | "query" | "search" | "searchquery") => "email_search_query",
        ("gmail", "from" | "sender") => "email_sender_filter",
        ("gmail", "subject") => "email_subject_filter",
        ("gmail", "label" | "labelid" | "labelids") => "email_label_filter",
        ("gmail", "messageid" | "threadid") => "email_reference",
        ("google_sheets", "range") => "sheet_range",
        ("google_sheets", "sheet" | "sheetname" | "worksheet") => "sheet_selector",
        ("google_sheets", "lookup" | "lookupvalue" | "lookupcolumn" | "filter") => "sheet_lookup",
        ("database", "query" | "sql" | "statement") => "database_read_query",
        (
            "database",
            "where" | "filter" | "lookup" | "lookupvalue" | "search" | "searchvalue" | "value",
        ) => "database_lookup",
        ("database", "table" | "tablename" | "collection") => "database_table",
        ("database", "from" | "to" | "date" | "startdate" | "enddate") => "database_date_filter",
        ("slack", "channel" | "channelid" | "room") => "slack_channel",
        ("slack", "text" | "message" | "body") => "slack_message",
        (_, "maxresults" | "limit") => "result_limit",
        _ => "app_parameter",
    }
}

fn app_candidate_reason(node_family: &str, operation_kind: &str, risk_hint: &str) -> &'static str {
    match (node_family, operation_kind, risk_hint) {
        ("gmail", "read", _) => {
            "Gmail read/search parameter can safely use prompt input while falling back to its current value."
        }
        ("google_sheets", "read", _) => {
            "Google Sheets read/lookup parameter can safely use prompt input while falling back to its current value."
        }
        ("database", "read", _) => {
            "Read-only database lookup parameter can safely use prompt input while falling back to its current value."
        }
        ("slack", "post_message", "yellow") => {
            "Slack message parameter can use prompt input, but posting is a side effect and requires explicit review."
        }
        _ => "App-node parameter can use prompt input while falling back to its current value.",
    }
}

fn app_test_value_hint(node_family: &str, field: &str) -> &'static str {
    match (node_family, field) {
        ("gmail", "email_query") => "is:unread newer_than:7d",
        ("gmail", "email_from") => "sender@example.com",
        ("gmail", "email_subject") => "project update",
        ("gmail", "email_label") => "INBOX",
        ("gmail", "message_id") => "gmail-message-id",
        ("google_sheets", "sheet_range") => "Sheet1!A1:D10",
        ("google_sheets", "sheet_name") => "Sheet1",
        ("google_sheets", "lookup_value") => "example",
        ("google_sheets", "lookup_column") => "Name",
        ("database", "database_query") => "select * from items where name = 'example'",
        ("database", "lookup_value") => "example",
        ("database", "table_name") => "items",
        ("database", "offset") => "0",
        ("database", "start_date") => "2026-01-01",
        ("database", "end_date") => "2026-12-31",
        ("slack", "slack_channel") => "#general",
        ("slack", "slack_message") => "Test message from KRIA",
        (_, "limit") => "10",
        _ => "",
    }
}

fn path_is_v1_editable(path: &[String]) -> bool {
    let joined = path.join(".").to_ascii_lowercase();
    joined.contains("queryparameters")
        || joined.contains("bodyparameters")
        || joined.contains("formparameters")
        || joined.contains("assignments")
        || joined.contains("values")
}

fn is_safe_static_value(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            !trimmed.is_empty()
                && !trimmed.contains("={{")
                && !trimmed.contains("$json")
                && !trimmed.contains("$input")
                && trimmed.len() <= 500
        }
        Value::Number(_) | Value::Bool(_) => true,
        _ => false,
    }
}

fn is_sensitive_key(text: &str) -> bool {
    [
        "api_key",
        "apikey",
        "authorization",
        "auth",
        "token",
        "secret",
        "password",
        "cookie",
        "session",
        "signature",
        "bearer",
        "client_secret",
        "access_key",
        "private_key",
        "headers",
        "credential",
    ]
    .iter()
    .any(|term| text.contains(term))
}

fn suggested_field_name(label: &str) -> String {
    match label.trim().to_ascii_lowercase().as_str() {
        "i" | "imdb" | "imdbid" | "imdb_id" => "imdb_id".into(),
        "t" | "title" | "movie" | "movie_title" => "title".into(),
        "q" | "query" | "search" | "keyword" => "query".into(),
        "type" => "type".into(),
        "y" | "year" => "year".into(),
        other => slugify(other),
    }
}

fn scalar_preview(value: &Value) -> String {
    match value {
        Value::String(text) => text.chars().take(120).collect(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        _ => "value".into(),
    }
}

fn recommended_fields(candidates: &[N8nInputParameterCandidate]) -> Vec<String> {
    dedupe(
        candidates
            .iter()
            .map(|candidate| candidate.suggested_field.clone())
            .collect(),
    )
}

fn expression_for_surface(
    surface: &N8nInputSurfaceType,
    field: &str,
    fallback_preview: &str,
) -> String {
    let accessor = json_accessor(field);
    let fallback = serde_json::to_string(fallback_preview).unwrap_or_else(|_| "\"\"".into());
    match surface {
        N8nInputSurfaceType::WebhookGet => {
            format!("={{{{ $json.query{accessor} ?? {fallback} }}}}")
        }
        N8nInputSurfaceType::WebhookPost => {
            format!("={{{{ $json.body{accessor} ?? $json{accessor} ?? {fallback} }}}}")
        }
        N8nInputSurfaceType::Form => format!("={{{{ $json{accessor} ?? {fallback} }}}}"),
        N8nInputSurfaceType::Chat => {
            if matches!(field, "query" | "prompt" | "text" | "title") {
                format!("={{{{ $json.chatInput ?? {fallback} }}}}")
            } else {
                format!("={{{{ $json{accessor} ?? {fallback} }}}}")
            }
        }
        _ => format!("={{{{ $json{accessor} ?? {fallback} }}}}"),
    }
}

fn json_accessor(field: &str) -> String {
    if is_js_identifier(field) {
        format!(".{field}")
    } else {
        format!(
            "[{}]",
            serde_json::to_string(field).unwrap_or_else(|_| "\"field\"".into())
        )
    }
}

fn is_js_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '$')
}

fn prepare_copy_workflow_json(
    workflow: &Value,
    copy_display_name: &str,
    copy_webhook_path: &str,
) -> Value {
    let mut copy = Value::Object(Map::new());
    if let Some(map) = copy.as_object_mut() {
        map.insert("name".into(), Value::String(copy_display_name.to_string()));
        map.insert(
            "nodes".into(),
            workflow
                .get("nodes")
                .cloned()
                .unwrap_or_else(|| Value::Array(Vec::new())),
        );
        map.insert(
            "connections".into(),
            workflow
                .get("connections")
                .cloned()
                .unwrap_or_else(|| Value::Object(Map::new())),
        );
        let execution_order = workflow
            .get("settings")
            .and_then(|settings| settings.get("executionOrder"))
            .and_then(Value::as_str)
            .unwrap_or("v1");
        map.insert(
            "settings".into(),
            serde_json::json!({
                "executionOrder": execution_order,
            }),
        );
    }
    if let Some(nodes) = copy.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            if let Some(map) = node.as_object_mut() {
                map.insert("id".into(), Value::String(uuid::Uuid::new_v4().to_string()));
            }
            let node_type = lower_node_type(node);
            if (node_type.contains("webhook") && !node_type.contains("respondtowebhook"))
                || node_type.contains("formtrigger")
                || node_type.contains("chattrigger")
            {
                if let Some(parameters) = node.get_mut("parameters").and_then(Value::as_object_mut)
                {
                    parameters.insert("path".into(), Value::String(copy_webhook_path.to_string()));
                    parameters.insert(
                        "webhookId".into(),
                        Value::String(copy_webhook_path.to_string()),
                    );
                    if node_type.contains("formtrigger") {
                        let options = parameters
                            .entry("options")
                            .or_insert_with(|| Value::Object(Map::new()));
                        if let Some(options) = options.as_object_mut() {
                            options.insert(
                                "path".into(),
                                Value::String(copy_webhook_path.to_string()),
                            );
                        }
                    }
                }
            }
        }
    }
    copy
}

fn set_node_parameter_value(
    workflow: &mut Value,
    node_name: &str,
    parameter_path: &[String],
    new_value: Value,
) -> Result<(), String> {
    let nodes = workflow
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| "workflow JSON does not contain nodes".to_string())?;
    let node = nodes
        .iter_mut()
        .find(|node| {
            node.get("name")
                .and_then(Value::as_str)
                .map(|name| name == node_name)
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("node '{node_name}' was not found in copied workflow"))?;
    let mut current = node
        .get_mut("parameters")
        .ok_or_else(|| format!("node '{node_name}' does not contain parameters"))?;
    for segment in parameter_path {
        if let Ok(index) = segment.parse::<usize>() {
            current = current
                .as_array_mut()
                .and_then(|array| array.get_mut(index))
                .ok_or_else(|| {
                    format!(
                        "parameter path '{}' was not found",
                        parameter_path.join(".")
                    )
                })?;
        } else {
            current = current
                .as_object_mut()
                .and_then(|object| object.get_mut(segment))
                .ok_or_else(|| {
                    format!(
                        "parameter path '{}' was not found",
                        parameter_path.join(".")
                    )
                })?;
        }
    }
    *current = new_value;
    Ok(())
}

fn build_input_schema(fields: &[String]) -> Value {
    let mut properties = Map::new();
    for field in fields {
        properties.insert(
            field.clone(),
            serde_json::json!({
                "type": "string",
                "description": format!("Input field mapped into the n8n workflow parameter '{}'.", field),
            }),
        );
    }
    properties.insert(
        "source_prompt".into(),
        serde_json::json!({
            "type": "string",
            "description": "Optional prompt text that caused this n8n workflow run.",
        }),
    );
    properties.insert(
        "confirmed_by_user".into(),
        serde_json::json!({
            "type": "boolean",
            "description": "Whether the user explicitly confirmed this run in KRIA.",
        }),
    );
    properties.insert(
        "kria_correlation_id".into(),
        serde_json::json!({
            "type": "string",
            "description": "KRIA correlation ID injected during execution.",
        }),
    );
    properties.insert(
        "kria_execution_id".into(),
        serde_json::json!({
            "type": "string",
            "description": "KRIA execution ID injected during execution.",
        }),
    );
    properties.insert(
        "kria_requested_by".into(),
        serde_json::json!({
            "type": "string",
            "description": "KRIA caller label injected during execution.",
        }),
    );
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    })
}

fn build_binary_input_schema(reports: &[N8nBinaryInputReport], fields: &[String]) -> Value {
    let mut properties = Map::new();
    for (index, field) in fields.iter().enumerate() {
        let report = reports.get(index);
        let mut mime_schema = serde_json::json!({ "type": "string" });
        if let Some(report) = report {
            if !report.accepted_mime_types.is_empty() {
                if let Some(map) = mime_schema.as_object_mut() {
                    map.insert(
                        "enum".into(),
                        Value::Array(
                            report
                                .accepted_mime_types
                                .iter()
                                .cloned()
                                .map(Value::String)
                                .collect(),
                        ),
                    );
                }
            }
        }
        properties.insert(
            field.clone(),
            serde_json::json!({
                "type": "object",
                "description": format!("Runtime-only file selected for '{}'. KRIA stores metadata only, not file contents.", field),
                "required": ["name", "size", "mime_type"],
                "properties": {
                    "name": { "type": "string" },
                    "size": {
                        "type": "integer",
                        "maximum": report.map(|item| item.max_size_bytes).unwrap_or(10 * 1024 * 1024)
                    },
                    "mime_type": mime_schema,
                    "sha256": { "type": "string" }
                }
            }),
        );
    }
    properties.insert(
        "__kria_files".into(),
        serde_json::json!({
            "type": "object",
            "additionalProperties": {
                "type": "object",
                "description": "Runtime-only file descriptor used by KRIA during a single test or run.",
                "properties": {
                    "name": { "type": "string" },
                    "size": { "type": "integer" },
                    "mime_type": { "type": "string" },
                    "sha256": { "type": "string" }
                }
            }
        }),
    );
    properties.insert(
        "source_prompt".into(),
        serde_json::json!({
            "type": "string",
            "description": "Optional prompt text that caused this n8n workflow run.",
        }),
    );
    properties.insert(
        "confirmed_by_user".into(),
        serde_json::json!({
            "type": "boolean",
            "description": "Whether the user explicitly confirmed this run in KRIA.",
        }),
    );
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    })
}

fn short_suffix(seed: &str) -> String {
    let digest = sha2::Sha256::digest(seed.as_bytes());
    hex::encode(digest)[..8].to_string()
}

fn sha256_hex(value: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(value.as_bytes()))
    )
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut last_was_sep = false;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('_');
            last_was_sep = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "workflow".into()
    } else {
        slug
    }
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(trimmed.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie_workflow_static() -> Value {
        serde_json::json!({
            "id": "wf_movies",
            "name": "Fetch Movies",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "fetch-movies"}
                },
                {
                    "name": "HTTP Request",
                    "type": "n8n-nodes-base.httpRequest",
                    "parameters": {
                        "method": "GET",
                        "url": "https://www.omdbapi.com/",
                        "sendQuery": true,
                        "queryParameters": {
                            "parameters": [
                                {"name": "apikey", "value": "secret"},
                                {"name": "i", "value": "tt3896198"},
                                {"name": "type", "value": "movie"}
                            ]
                        }
                    }
                }
            ],
            "connections": {
                "Webhook": {"main": [[{"node": "HTTP Request", "type": "main", "index": 0}]]}
            }
        })
    }

    #[test]
    fn detects_webhook_workflow_that_ignores_input() {
        let report = analyze_n8n_input_capability(&movie_workflow_static());

        assert_eq!(
            report.input_capability,
            N8nInputCapability::InputReceivesButIgnores
        );
        assert_eq!(report.input_surface_type, N8nInputSurfaceType::WebhookPost);
        assert!(report.recommended_input_fields.contains(&"imdb_id".into()));
        assert!(report.recommended_input_fields.contains(&"type".into()));
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("Skipped sensitive")));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .all(|candidate| candidate.parameter_label != "apikey"));
    }

    #[test]
    fn detects_webhook_workflow_that_already_uses_input() {
        let mut workflow = movie_workflow_static();
        workflow["nodes"][1]["parameters"]["queryParameters"]["parameters"][1]["value"] =
            serde_json::json!("={{ $json.body.title }}");

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(report.input_capability, N8nInputCapability::InputReady);
    }

    #[test]
    fn schedule_workflow_has_no_v1_input_surface() {
        let workflow = serde_json::json!({
            "name": "Scheduled Mail",
            "nodes": [{
                "name": "Schedule Trigger",
                "type": "n8n-nodes-base.scheduleTrigger",
                "parameters": {}
            }]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(report.input_capability, N8nInputCapability::NoInputSurface);
    }

    #[test]
    fn gmail_read_search_node_creates_safe_candidates() {
        let workflow = serde_json::json!({
            "id": "wf_gmail",
            "name": "Gmail Search",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "gmail-search"}
                },
                {
                    "name": "Gmail",
                    "type": "n8n-nodes-base.gmail",
                    "parameters": {
                        "resource": "message",
                        "operation": "getAll",
                        "q": "is:unread",
                        "maxResults": 5
                    },
                    "credentials": {"gmailOAuth2": {"id": "credential-id", "name": "Gmail"}}
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.input_capability,
            N8nInputCapability::InputReceivesButIgnores
        );
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.node_family == "gmail"
                && candidate.suggested_field == "email_query"
                && candidate.risk_hint == "green"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.suggested_field == "limit"));
    }

    #[test]
    fn gmail_write_operations_are_not_adapted() {
        let workflow = serde_json::json!({
            "name": "Gmail Send",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "gmail-send"}
                },
                {
                    "name": "Gmail",
                    "type": "n8n-nodes-base.gmail",
                    "parameters": {
                        "resource": "message",
                        "operation": "send",
                        "to": "user@example.com",
                        "subject": "Hello"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert!(report.hardcoded_parameter_candidates.is_empty());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("Gmail write/update operation")));
    }

    #[test]
    fn sheets_read_node_creates_safe_candidates_and_skips_spreadsheet_id() {
        let workflow = serde_json::json!({
            "name": "Sheet Lookup",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "sheet-lookup"}
                },
                {
                    "name": "Google Sheets",
                    "type": "n8n-nodes-base.googleSheets",
                    "parameters": {
                        "resource": "sheet",
                        "operation": "read",
                        "spreadsheetId": "secret-sheet-id",
                        "range": "Sheet1!A1:D10",
                        "lookupValue": "Alice"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.node_family == "google_sheets"
                && candidate.suggested_field == "sheet_range"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.suggested_field == "lookup_value"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .all(|candidate| candidate.parameter_label != "spreadsheetId"));
    }

    #[test]
    fn sheets_write_operations_are_not_adapted() {
        let workflow = serde_json::json!({
            "name": "Sheet Append",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "sheet-append"}
                },
                {
                    "name": "Google Sheets",
                    "type": "n8n-nodes-base.googleSheets",
                    "parameters": {
                        "resource": "sheet",
                        "operation": "append",
                        "range": "Sheet1!A1:D10"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert!(report.hardcoded_parameter_candidates.is_empty());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("Google Sheets write/update operation")));
    }

    #[test]
    fn slack_post_node_is_yellow_and_requires_confirmation() {
        let workflow = serde_json::json!({
            "name": "Slack Post",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "slack-post"}
                },
                {
                    "name": "Slack",
                    "type": "n8n-nodes-base.slack",
                    "parameters": {
                        "resource": "message",
                        "operation": "post",
                        "channel": "#general",
                        "text": "Build passed",
                        "token": "secret"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.node_family == "slack"
                && candidate.requires_strong_confirmation
                && candidate.risk_hint == "yellow"
                && candidate.suggested_field == "slack_message"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .all(|candidate| candidate.parameter_label != "token"));
    }

    #[test]
    fn database_read_select_node_creates_safe_candidates() {
        let workflow = serde_json::json!({
            "id": "wf_db",
            "name": "Database Lookup",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "database-lookup"}
                },
                {
                    "name": "Postgres",
                    "type": "n8n-nodes-base.postgres",
                    "parameters": {
                        "operation": "executeQuery",
                        "query": "SELECT id, email FROM users WHERE email = :email",
                        "where": "alice@example.com",
                        "limit": 10,
                        "host": "db.internal",
                        "password": "secret"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.input_capability,
            N8nInputCapability::InputReceivesButIgnores
        );
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.node_family == "database"
                && candidate.suggested_field == "lookup_value"
                && candidate.risk_hint == "green"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .any(|candidate| candidate.suggested_field == "limit"));
        assert!(report
            .hardcoded_parameter_candidates
            .iter()
            .all(|candidate| !matches!(
                candidate.parameter_label.as_str(),
                "host" | "password" | "query"
            )));
    }

    #[test]
    fn database_write_operations_are_not_adapted() {
        for operation in ["insert", "update", "delete", "upsert", "drop", "truncate"] {
            let workflow = serde_json::json!({
                "name": format!("Database {operation}"),
                "nodes": [
                    {
                        "name": "Webhook",
                        "type": "n8n-nodes-base.webhook",
                        "parameters": {"httpMethod": "POST", "path": "database-write"}
                    },
                    {
                        "name": "Postgres",
                        "type": "n8n-nodes-base.postgres",
                        "parameters": {
                            "operation": operation,
                            "where": "alice@example.com"
                        }
                    }
                ]
            });

            let report = analyze_n8n_input_capability(&workflow);

            assert!(
                report.hardcoded_parameter_candidates.is_empty(),
                "operation {operation} should not create candidates"
            );
            assert!(report
                .warnings
                .iter()
                .any(|warning| warning.contains("database write/admin operation")));
        }
    }

    #[test]
    fn database_sql_scanner_accepts_simple_read_only_select() {
        assert_eq!(
            sql_read_safety("SELECT id, email FROM users WHERE email = :email"),
            SqlReadSafety::ReadOnly
        );
        assert_eq!(
            sql_read_safety(
                "WITH active_users AS (SELECT * FROM users) SELECT * FROM active_users"
            ),
            SqlReadSafety::ReadOnly
        );
    }

    #[test]
    fn database_sql_scanner_blocks_multi_statement_hidden_write_and_ddl() {
        for sql in [
            "SELECT * FROM users; DELETE FROM users",
            "SELECT * FROM users -- DELETE FROM users",
            "SELECT * FROM users /* hidden */",
            "DROP TABLE users",
            "CALL rotate_admin_keys()",
        ] {
            assert!(
                matches!(sql_read_safety(sql), SqlReadSafety::Unsafe(_)),
                "{sql} should be unsafe"
            );
        }
    }

    #[test]
    fn database_uncertain_operation_needs_input_review() {
        let workflow = serde_json::json!({
            "name": "Database Unknown",
            "nodes": [
                {
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "database-unknown"}
                },
                {
                    "name": "Postgres",
                    "type": "n8n-nodes-base.postgres",
                    "parameters": {
                        "mode": "custom",
                        "where": "alice@example.com"
                    }
                }
            ]
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.input_capability,
            N8nInputCapability::NeedsInputReview
        );
        assert!(report.hardcoded_parameter_candidates.is_empty());
        assert!(report
            .warnings
            .iter()
            .any(|warning| warning.contains("could not verify a read/select operation")));
    }

    fn code_workflow(js_code: &str) -> Value {
        serde_json::json!({
            "id": "wf_code",
            "name": "Code Movie",
            "nodes": [
                {
                    "id": "webhook",
                    "name": "Webhook",
                    "type": "n8n-nodes-base.webhook",
                    "parameters": {"httpMethod": "POST", "path": "code-movie"}
                },
                {
                    "id": "code",
                    "name": "Code",
                    "type": "n8n-nodes-base.code",
                    "parameters": {"mode": "runOnceForAllItems", "jsCode": js_code}
                }
            ],
            "connections": {
                "Webhook": {"main": [[{"node": "Code", "type": "main", "index": 0}]]}
            }
        })
    }

    #[test]
    fn code_node_using_json_body_is_input_ready() {
        let workflow = code_workflow(
            r#"const title = $json.body.title;
return [{ json: { title } }];"#,
        );
        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(report.input_capability, N8nInputCapability::InputReady);
        assert_eq!(
            report.code_node_reports[0].classification,
            N8nCodeNodeClassification::InputReady
        );
    }

    #[test]
    fn code_node_using_input_first_is_input_ready() {
        let workflow = code_workflow(
            r#"const query = $input.first().json.query;
return [{ json: { query } }];"#,
        );
        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(report.input_capability, N8nInputCapability::InputReady);
        assert!(report.code_node_reports[0]
            .input_references
            .contains(&"$input".into()));
    }

    #[test]
    fn hardcoded_code_return_object_gets_patch_preview() {
        let workflow = code_workflow(
            r#"return [{ json: { title: "Guardians of the Galaxy", year: 2017, enabled: true } }];"#,
        );
        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.input_capability,
            N8nInputCapability::NeedsInputReview
        );
        assert_eq!(
            report.code_node_reports[0].classification,
            N8nCodeNodeClassification::PatchPreviewAvailable
        );
        assert!(report.code_node_reports[0]
            .hardcoded_literals
            .iter()
            .any(|hint| hint.suggested_field == "title"));
    }

    #[test]
    fn hardcoded_const_code_gets_patch_and_preserves_original() {
        let workflow = code_workflow(
            r#"const title = "Guardians of the Galaxy";
const limit = 5;
return [{ json: { title, limit } }];"#,
        );
        let original = workflow.clone();
        let plan = build_n8n_code_input_aware_copy_plan(
            &workflow,
            "code_movie_input",
            "Code Movie - KRIA Code Input Version",
            &[],
        );

        assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
        assert!(plan.accepted_fields.contains(&"title".into()));
        assert!(plan.accepted_fields.contains(&"limit".into()));
        assert_ne!(
            plan.workflow_json["nodes"][1]["parameters"]["jsCode"],
            workflow["nodes"][1]["parameters"]["jsCode"]
        );
        assert_eq!(workflow, original);
        let patched = plan.workflow_json["nodes"][1]["parameters"]["jsCode"]
            .as_str()
            .unwrap();
        assert!(patched.contains("__kriaInput"));
        assert!(patched.contains("Guardians of the Galaxy"));
        assert!(patched.contains("Number("));
    }

    #[test]
    fn unsafe_code_blocks_auto_patch() {
        let workflow = code_workflow(
            r#"const token = process.env.API_TOKEN;
eval("console.log(token)");
return [{ json: { token } }];"#,
        );
        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.code_node_reports[0].classification,
            N8nCodeNodeClassification::UnsafeBlocked
        );
        let plan = build_n8n_code_input_aware_copy_plan(
            &workflow,
            "unsafe_code_input",
            "Unsafe Code Input",
            &[],
        );
        assert!(!plan.blockers.is_empty());
    }

    #[test]
    fn complex_code_returns_manual_review() {
        let workflow = code_workflow(
            r#"for (const item of items) {
  item.json.title = "Guardians";
}
return items;"#,
        );
        let report = analyze_n8n_input_capability(&workflow);

        assert!(matches!(
            report.code_node_reports[0].classification,
            N8nCodeNodeClassification::InputReady | N8nCodeNodeClassification::ManualReviewRequired
        ));
        assert_ne!(report.code_node_reports[0].patch_eligibility, "auto_patch");
    }

    #[test]
    fn detects_form_trigger_file_input() {
        let workflow = serde_json::json!({
            "id": "wf_file",
            "name": "Upload Contract",
            "nodes": [
                {
                    "id": "form",
                    "name": "Form",
                    "type": "n8n-nodes-base.formTrigger",
                    "parameters": {
                        "path": "upload-contract",
                        "formFields": {
                            "values": [
                                {"fieldLabel": "Contract File", "fieldType": "file", "requiredField": true},
                                {"fieldLabel": "Title", "fieldType": "text"}
                            ]
                        }
                    }
                },
                {
                    "id": "http",
                    "name": "HTTP Request",
                    "type": "n8n-nodes-base.httpRequest",
                    "parameters": {"method": "POST", "url": "https://example.com/upload"}
                }
            ],
            "connections": {
                "Form": {"main": [[{"node": "HTTP Request", "type": "main", "index": 0}]]}
            }
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert_eq!(
            report.v5_capability_status,
            N8nV5CapabilityStatus::FileReady
        );
        assert_eq!(report.binary_input_reports.len(), 1);
        assert_eq!(report.binary_input_reports[0].input_kind, "form_file");
        assert_eq!(report.binary_input_reports[0].field_name, "field-0");
        assert!(report.binary_input_reports[0].safe);
    }

    #[test]
    fn detects_branch_output_review_need() {
        let workflow = serde_json::json!({
            "id": "wf_branch",
            "name": "Branch Result",
            "nodes": [
                {"id": "webhook", "name": "Webhook", "type": "n8n-nodes-base.webhook", "parameters": {"httpMethod": "POST", "path": "branch"}},
                {"id": "if", "name": "IF", "type": "n8n-nodes-base.if", "parameters": {}},
                {"id": "ok", "name": "Success HTTP", "type": "n8n-nodes-base.httpRequest", "parameters": {"url": "https://example.com/ok"}},
                {"id": "fallback", "name": "Fallback Code", "type": "n8n-nodes-base.code", "parameters": {"jsCode": "return [{json:{result:'fallback'}}];"}}
            ],
            "connections": {
                "Webhook": {"main": [[{"node": "IF", "type": "main", "index": 0}]]},
                "IF": {"main": [
                    [{"node": "Success HTTP", "type": "main", "index": 0}],
                    [{"node": "Fallback Code", "type": "main", "index": 0}]
                ]}
            }
        });

        let report = analyze_n8n_input_capability(&workflow);

        assert!(!report.branch_reports.is_empty());
        assert!(report.output_selection_report.preferred_required);
        assert!(report.output_selection_report.candidates.len() >= 2);
    }

    #[test]
    fn binary_copy_plan_requires_preferred_output_when_ambiguous() {
        let workflow = serde_json::json!({
            "id": "wf_file_branch",
            "name": "File Branch",
            "nodes": [
                {
                    "id": "form",
                    "name": "Form",
                    "type": "n8n-nodes-base.formTrigger",
                    "parameters": {
                        "path": "file-branch",
                        "formFields": {
                            "values": [
                                {"fieldLabel": "Attachment", "fieldType": "file"}
                            ]
                        }
                    }
                },
                {"id": "if", "name": "IF", "type": "n8n-nodes-base.if", "parameters": {}},
                {"id": "ok", "name": "OK HTTP", "type": "n8n-nodes-base.httpRequest", "parameters": {"url": "https://example.com/ok"}},
                {"id": "review", "name": "Review Code", "type": "n8n-nodes-base.code", "parameters": {"jsCode": "return [{json:{review:true}}];"}}
            ],
            "connections": {
                "Form": {"main": [[{"node": "IF", "type": "main", "index": 0}]]},
                "IF": {"main": [
                    [{"node": "OK HTTP", "type": "main", "index": 0}],
                    [{"node": "Review Code", "type": "main", "index": 0}]
                ]}
            }
        });

        let blocked = build_n8n_binary_input_aware_copy_plan(
            &workflow,
            "file_branch_input",
            "File Branch - KRIA File Input Version",
            &[],
            None,
        );
        assert!(blocked
            .blockers
            .iter()
            .any(|blocker| blocker.contains("Choose a preferred output node")));

        let allowed = build_n8n_binary_input_aware_copy_plan(
            &workflow,
            "file_branch_input",
            "File Branch - KRIA File Input Version",
            &[],
            Some("OK HTTP"),
        );
        assert!(allowed.blockers.is_empty(), "{:?}", allowed.blockers);
        assert_eq!(workflow["name"], "File Branch");
        assert_eq!(
            allowed.workflow_json["name"],
            "File Branch - KRIA File Input Version"
        );
        assert!(allowed.input_schema["properties"]["field-0"].is_object());
    }

    #[test]
    fn builds_input_aware_copy_without_mutating_original() {
        let original = movie_workflow_static();
        let plan = build_n8n_input_aware_copy_plan(
            &original,
            "fetch_movies_input",
            "Fetch Movies - KRIA Input Version",
            &[N8nInputAwareMappingReview {
                mapping_id: "http_request_queryparameters_parameters_1_value_imdb_id".into(),
                field_name: "title".into(),
                accepted: true,
                custom_expression: String::new(),
            }],
        );

        assert!(plan.blockers.is_empty());
        assert_eq!(original["name"], "Fetch Movies");
        assert_eq!(
            plan.workflow_json["name"],
            "Fetch Movies - KRIA Input Version"
        );
        assert!(plan.workflow_json["id"].is_null());
        assert!(plan.workflow_json.to_string().contains("$json.body.title"));
        assert!(plan
            .workflow_json
            .to_string()
            .contains("={{ $json.body.title"));
        assert_eq!(plan.input_schema["properties"]["title"]["type"], "string");
    }
}
