use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Serialize;
use tree_sitter::Node;

use crate::config::{
    DiffConfig, DiffFactKindConfig, DiffMetricThresholds, DiffRiskConfig, DiffRiskMatch,
    DiffRiskSeverity, DiffThreshold,
};
use crate::discovery::{ChangeKind, ChangedFile, ChangedWorkspace, DetectedVcs};
use crate::parser::{MatlabParser, ParseResult, ParserError};
use crate::preserve::{PreservationReason, collect_preservation_zones};
use crate::source::{ByteRange, PositionError, SourceFile, SourcePosition};

const LCS_CELL_LIMIT: usize = 2_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffSide {
    Old,
    New,
}

impl fmt::Display for DiffSide {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Old => "old",
            Self::New => "new",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseConfidence {
    Full,
    Recovered,
    Unavailable,
}

impl fmt::Display for ParseConfidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Full => "full",
            Self::Recovered => "recovered",
            Self::Unavailable => "unavailable",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileClass {
    Implementation,
    Test,
}

impl fmt::Display for FileClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Implementation => "implementation",
            Self::Test => "test",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticFactKind {
    FunctionAdded,
    FunctionDeleted,
    FunctionSignatureChanged,
    PublicApiSignatureChanged,
    PersistentChanged,
    GlobalChanged,
    AssignmentChanged,
    NumericExpressionChanged,
    CallOrIndexChanged,
    ControlFlowChanged,
    SectionAdded,
    SectionTouched,
}

impl fmt::Display for SemanticFactKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::FunctionAdded => "function_added",
            Self::FunctionDeleted => "function_deleted",
            Self::FunctionSignatureChanged => "function_signature_changed",
            Self::PublicApiSignatureChanged => "public_api_signature_changed",
            Self::PersistentChanged => "persistent_changed",
            Self::GlobalChanged => "global_changed",
            Self::AssignmentChanged => "assignment_changed",
            Self::NumericExpressionChanged => "numeric_expression_changed",
            Self::CallOrIndexChanged => "call_or_index_changed",
            Self::ControlFlowChanged => "control_flow_changed",
            Self::SectionAdded => "section_added",
            Self::SectionTouched => "section_touched",
        })
    }
}

impl From<DiffFactKindConfig> for SemanticFactKind {
    fn from(value: DiffFactKindConfig) -> Self {
        match value {
            DiffFactKindConfig::FunctionAdded => Self::FunctionAdded,
            DiffFactKindConfig::FunctionDeleted => Self::FunctionDeleted,
            DiffFactKindConfig::FunctionSignatureChanged => Self::FunctionSignatureChanged,
            DiffFactKindConfig::PublicApiSignatureChanged => Self::PublicApiSignatureChanged,
            DiffFactKindConfig::PersistentChanged => Self::PersistentChanged,
            DiffFactKindConfig::GlobalChanged => Self::GlobalChanged,
            DiffFactKindConfig::AssignmentChanged => Self::AssignmentChanged,
            DiffFactKindConfig::NumericExpressionChanged => Self::NumericExpressionChanged,
            DiffFactKindConfig::CallOrIndexChanged => Self::CallOrIndexChanged,
            DiffFactKindConfig::ControlFlowChanged => Self::ControlFlowChanged,
            DiffFactKindConfig::SectionAdded => Self::SectionAdded,
            DiffFactKindConfig::SectionTouched => Self::SectionTouched,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRange {
    pub start: SourcePosition,
    pub end: SourcePosition,
    pub start_byte: usize,
    pub end_byte: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SemanticFact {
    pub kind: SemanticFactKind,
    pub change: ChangeKind,
    pub side: DiffSide,
    pub path: String,
    pub range: SourceRange,
    pub enclosing_function: Option<String>,
    pub identifiers: Vec<String>,
    pub call_or_index_targets: Vec<String>,
    pub field_paths: Vec<String>,
    pub operators: Vec<String>,
    pub literals: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FunctionChangeKind {
    Touched,
    Added,
    Deleted,
    SignatureChanged,
}

impl fmt::Display for FunctionChangeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Touched => "touched",
            Self::Added => "added",
            Self::Deleted => "deleted",
            Self::SignatureChanged => "signature_changed",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FunctionChange {
    pub name: String,
    pub change: FunctionChangeKind,
    pub old_range: Option<SourceRange>,
    pub new_range: Option<SourceRange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct DiffMetrics {
    pub added_executable_lines: usize,
    pub deleted_executable_lines: usize,
    pub files_touched: usize,
    pub functions_touched: usize,
    pub functions_added: usize,
    pub control_flow_increase: usize,
    pub sections_touched: usize,
    pub sections_added: usize,
}

impl DiffMetrics {
    fn add_assign(&mut self, other: Self) {
        self.added_executable_lines += other.added_executable_lines;
        self.deleted_executable_lines += other.deleted_executable_lines;
        self.files_touched += other.files_touched;
        self.functions_touched += other.functions_touched;
        self.functions_added += other.functions_added;
        self.control_flow_increase += other.control_flow_increase;
        self.sections_touched += other.sections_touched;
        self.sections_added += other.sections_added;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileAnalysis {
    pub old_path: Option<String>,
    pub new_path: Option<String>,
    pub classification: FileClass,
    pub parse_confidence: ParseConfidence,
    pub metrics: DiffMetrics,
    pub embedded_test_regions_touched: usize,
    pub functions: Vec<FunctionChange>,
    pub facts: Vec<SemanticFact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileAnalysis {
    pub name: String,
    pub metrics: DiffMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum DiffDecision {
    #[serde(rename = "ACCEPT")]
    Accept,
    #[serde(rename = "REVIEW")]
    Review,
    #[serde(rename = "REJECT / SPLIT")]
    RejectSplit,
}

impl fmt::Display for DiffDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept => formatter.write_str("ACCEPT"),
            Self::Review => formatter.write_str("REVIEW"),
            Self::RejectSplit => formatter.write_str("REJECT / SPLIT"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PolicyTrigger {
    pub id: String,
    pub severity: DiffDecision,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiffAnalysis {
    pub vcs: DetectedVcs,
    pub advisory: bool,
    pub decision: DiffDecision,
    pub source_diagnostics: bool,
    pub metrics: DiffMetrics,
    pub test_metrics: DiffMetrics,
    pub profiles: Vec<ProfileAnalysis>,
    pub generated_files: Vec<String>,
    pub triggers: Vec<PolicyTrigger>,
    pub files: Vec<FileAnalysis>,
}

pub fn analyze_diff(
    workspace: &ChangedWorkspace,
    config: &DiffConfig,
) -> Result<DiffAnalysis, DiffError> {
    let generated_matcher = PathMatcher::new(
        &workspace.root,
        &config.generated_paths,
        "diff.generated_paths",
    )?;
    let test_matcher = PathMatcher::new(&workspace.root, &config.test_paths, "diff.test_paths")?;
    let public_api = load_public_api_inventory(&workspace.root, config)?;
    let mut parser = MatlabParser::new().map_err(DiffError::Parser)?;

    let mut files = Vec::new();
    let mut generated_files = Vec::new();
    for changed in &workspace.files {
        if !changed_has_matlab_path(changed) {
            continue;
        }
        if selected_path_matches(changed, &generated_matcher) {
            generated_files.push(display_changed_path(changed));
            continue;
        }
        let classification = if selected_path_matches(changed, &test_matcher) {
            FileClass::Test
        } else {
            FileClass::Implementation
        };
        files.push(analyze_file(
            changed,
            classification,
            &public_api,
            &mut parser,
        )?);
    }

    files.sort_by(|left, right| {
        left.new_path
            .as_ref()
            .or(left.old_path.as_ref())
            .cmp(&right.new_path.as_ref().or(right.old_path.as_ref()))
    });
    generated_files.sort();
    generated_files.dedup();

    let mut metrics = DiffMetrics::default();
    let mut test_metrics = DiffMetrics::default();
    for file in &files {
        match file.classification {
            FileClass::Implementation => metrics.add_assign(file.metrics),
            FileClass::Test => test_metrics.add_assign(file.metrics),
        }
    }

    let mut decision = DiffDecision::Accept;
    let mut triggers = Vec::new();
    evaluate_thresholds(
        "general",
        metrics,
        &config.thresholds.general,
        &mut decision,
        &mut triggers,
    );

    let mut profiles = Vec::new();
    for profile in &config.thresholds.profile {
        let matcher = PathMatcher::new(
            &workspace.root,
            &profile.paths,
            &format!("diff.thresholds.profile {:?}", profile.name),
        )?;
        let mut profile_metrics = DiffMetrics::default();
        for file in &files {
            if file.classification == FileClass::Implementation
                && file_analysis_matches(file, &matcher)
            {
                profile_metrics.add_assign(file.metrics);
            }
        }
        if profile_metrics.files_touched > 0 {
            let profile_thresholds = profile.metrics();
            evaluate_thresholds(
                &format!("profile:{}", profile.name),
                profile_metrics,
                &profile_thresholds,
                &mut decision,
                &mut triggers,
            );
            profiles.push(ProfileAnalysis {
                name: profile.name.clone(),
                metrics: profile_metrics,
            });
        }
    }

    let source_diagnostics = files
        .iter()
        .any(|file| file.parse_confidence == ParseConfidence::Recovered);
    if source_diagnostics {
        raise_decision(&mut decision, DiffDecision::Review);
        triggers.push(PolicyTrigger {
            id: "PARSE_CONFIDENCE".to_owned(),
            severity: DiffDecision::Review,
            scope: "changed_source".to_owned(),
            metric: None,
            value: None,
            threshold: None,
        });
    }

    evaluate_risks(
        &workspace.root,
        &files,
        &config.risk,
        &mut decision,
        &mut triggers,
    )?;

    triggers.sort_by(|left, right| {
        (
            &left.id,
            left.severity,
            &left.scope,
            &left.metric,
            left.value,
        )
            .cmp(&(
                &right.id,
                right.severity,
                &right.scope,
                &right.metric,
                right.value,
            ))
    });
    triggers.dedup_by(|left, right| {
        left.id == right.id
            && left.severity == right.severity
            && left.scope == right.scope
            && left.metric == right.metric
            && left.value == right.value
            && left.threshold == right.threshold
    });

    Ok(DiffAnalysis {
        vcs: workspace.vcs,
        advisory: config.advisory,
        decision,
        source_diagnostics,
        metrics,
        test_metrics,
        profiles,
        generated_files,
        triggers,
        files,
    })
}

#[allow(clippy::collapsible_if)]
fn analyze_file(
    changed: &ChangedFile,
    classification: FileClass,
    public_api: &BTreeSet<String>,
    parser: &mut MatlabParser,
) -> Result<FileAnalysis, DiffError> {
    let old_path = changed.old_path.as_deref();
    let new_path = changed.new_path.as_deref();
    let old_file = changed.old_source.as_ref().map(|source| {
        SourceFile::new(
            old_path.unwrap_or_else(|| Path::new("<old>")),
            source.clone(),
        )
    });
    let new_file = changed.new_source.as_ref().map(|source| {
        SourceFile::new(
            new_path.unwrap_or_else(|| Path::new("<new>")),
            source.clone(),
        )
    });

    let old_parse = old_file
        .as_ref()
        .map(|source| parser.parse(source).map_err(DiffError::Parser))
        .transpose()?;
    let new_parse = new_file
        .as_ref()
        .map(|source| parser.parse(source).map_err(DiffError::Parser))
        .transpose()?;

    let old_lines = line_table(old_file.as_ref().map_or("", SourceFile::text));
    let new_lines = line_table(new_file.as_ref().map_or("", SourceFile::text));
    let chunks = diff_chunks(&old_lines, &new_lines);
    let old_executable = executable_line_flags(&old_lines);
    let new_executable = executable_line_flags(&new_lines);
    let mut old_changed_all = BTreeSet::new();
    let mut new_changed_all = BTreeSet::new();
    let mut old_changed_exec = BTreeSet::new();
    let mut new_changed_exec = BTreeSet::new();

    for chunk in &chunks {
        old_changed_all.extend(chunk.old_start..chunk.old_end);
        new_changed_all.extend(chunk.new_start..chunk.new_end);

        for refined in formatting_equivalence_chunks(*chunk) {
            let formatting_only = old_file
                .as_ref()
                .zip(old_parse.as_ref())
                .zip(new_file.as_ref().zip(new_parse.as_ref()))
                .is_some_and(|((old_source, old_parsed), (new_source, new_parsed))| {
                    replacement_is_syntax_equivalent(
                        old_source,
                        old_parsed,
                        &old_lines,
                        refined.old_start,
                        refined.old_end,
                        new_source,
                        new_parsed,
                        &new_lines,
                        refined.new_start,
                        refined.new_end,
                    )
                });
            if formatting_only {
                continue;
            }

            for index in refined.old_start..refined.old_end {
                if old_executable.get(index).copied().unwrap_or(false) {
                    old_changed_exec.insert(index);
                }
            }
            for index in refined.new_start..refined.new_end {
                if new_executable.get(index).copied().unwrap_or(false) {
                    new_changed_exec.insert(index);
                }
            }
        }
    }

    let old_exec_ranges = line_indices_to_ranges(
        &old_lines,
        &old_changed_exec,
        old_file.as_ref().map_or(0, SourceFile::len),
    );
    let new_exec_ranges = line_indices_to_ranges(
        &new_lines,
        &new_changed_exec,
        new_file.as_ref().map_or(0, SourceFile::len),
    );
    let old_exec_line_ranges = line_indices_to_single_line_ranges(
        &old_lines,
        &old_changed_exec,
        old_file.as_ref().map_or(0, SourceFile::len),
    );
    let new_exec_line_ranges = line_indices_to_single_line_ranges(
        &new_lines,
        &new_changed_exec,
        new_file.as_ref().map_or(0, SourceFile::len),
    );

    let old_unsafe_ranges = old_parse
        .as_ref()
        .map(|parsed| recovery_ranges(parsed, &old_exec_ranges))
        .unwrap_or_default();
    let new_unsafe_ranges = new_parse
        .as_ref()
        .map(|parsed| recovery_ranges(parsed, &new_exec_ranges))
        .unwrap_or_default();
    let old_recovery = !old_unsafe_ranges.is_empty();
    let new_recovery = !new_unsafe_ranges.is_empty();
    let parse_confidence = if old_file.is_none() && new_file.is_none() {
        ParseConfidence::Unavailable
    } else if old_recovery || new_recovery {
        ParseConfidence::Recovered
    } else {
        ParseConfidence::Full
    };

    let old_functions = old_file
        .as_ref()
        .zip(old_parse.as_ref())
        .map(|(source, parsed)| collect_functions(source, parsed))
        .transpose()?
        .unwrap_or_default();
    let new_functions = new_file
        .as_ref()
        .zip(new_parse.as_ref())
        .map(|(source, parsed)| collect_functions(source, parsed))
        .transpose()?
        .unwrap_or_default();

    let old_function_map = old_functions
        .iter()
        .map(|function| (function.key.clone(), function))
        .collect::<BTreeMap<_, _>>();
    let new_function_map = new_functions
        .iter()
        .map(|function| (function.key.clone(), function))
        .collect::<BTreeMap<_, _>>();

    let mut touched_keys = BTreeSet::new();
    collect_smallest_touched_functions(&old_functions, &old_exec_line_ranges, &mut touched_keys);
    collect_smallest_touched_functions(&new_functions, &new_exec_line_ranges, &mut touched_keys);

    let added_keys = new_function_map
        .keys()
        .filter(|key| !old_function_map.contains_key(*key))
        .cloned()
        .collect::<BTreeSet<_>>();
    let deleted_keys = old_function_map
        .keys()
        .filter(|key| !new_function_map.contains_key(*key))
        .cloned()
        .collect::<BTreeSet<_>>();
    let signature_changed_keys = old_function_map
        .iter()
        .filter_map(|(key, old)| {
            new_function_map
                .get(key)
                .filter(|new| new.signature_fingerprint != old.signature_fingerprint)
                .map(|_| key.clone())
        })
        .collect::<BTreeSet<_>>();
    let old_primary = primary_function(&old_functions);
    let new_primary = primary_function(&new_functions);
    let primary_signature_changed = old_primary
        .zip(new_primary)
        .is_some_and(|(old, new)| old.signature_fingerprint != new.signature_fingerprint);

    let old_sections = collect_sections(&old_lines);
    let new_sections = collect_sections(&new_lines);
    let old_sections_touched = sections_touched(&old_sections, &old_changed_all);
    let new_sections_touched = sections_touched(&new_sections, &new_changed_all);
    let sections_touched_count = old_sections_touched.len().max(new_sections_touched.len());
    let added_section_indices = added_sections_by_chunks(&chunks, &old_sections, &new_sections);
    let old_embedded_tests = collect_embedded_test_regions(&old_lines);
    let new_embedded_tests = collect_embedded_test_regions(&new_lines);
    let embedded_test_regions_touched = sections_touched(&old_embedded_tests, &old_changed_all)
        .len()
        .max(sections_touched(&new_embedded_tests, &new_changed_all).len());

    let structural_metrics_available = parse_confidence != ParseConfidence::Recovered;
    let old_control_flow = if structural_metrics_available {
        old_parse.as_ref().map_or(0, |parsed| {
            count_control_flow_in_ranges(parsed.tree().root_node(), &old_exec_ranges)
        })
    } else {
        0
    };
    let new_control_flow = if structural_metrics_available {
        new_parse.as_ref().map_or(0, |parsed| {
            count_control_flow_in_ranges(parsed.tree().root_node(), &new_exec_ranges)
        })
    } else {
        0
    };

    let metrics = DiffMetrics {
        added_executable_lines: new_changed_exec.len(),
        deleted_executable_lines: old_changed_exec.len(),
        files_touched: 1,
        functions_touched: if structural_metrics_available {
            touched_keys.len()
        } else {
            0
        },
        functions_added: if structural_metrics_available {
            added_keys.len()
        } else {
            0
        },
        control_flow_increase: new_control_flow.saturating_sub(old_control_flow),
        sections_touched: sections_touched_count,
        sections_added: added_section_indices.len(),
    };

    let mut functions = Vec::new();
    for key in &touched_keys {
        let old = old_function_map.get(key).copied();
        let new = new_function_map.get(key).copied();
        functions.push(function_change(
            old,
            new,
            FunctionChangeKind::Touched,
            old_file.as_ref(),
            new_file.as_ref(),
        )?);
    }
    for key in &added_keys {
        let new = new_function_map.get(key).copied();
        functions.push(function_change(
            None,
            new,
            FunctionChangeKind::Added,
            old_file.as_ref(),
            new_file.as_ref(),
        )?);
    }
    for key in &deleted_keys {
        let old = old_function_map.get(key).copied();
        functions.push(function_change(
            old,
            None,
            FunctionChangeKind::Deleted,
            old_file.as_ref(),
            new_file.as_ref(),
        )?);
    }
    for key in &signature_changed_keys {
        let old = old_function_map.get(key).copied();
        let new = new_function_map.get(key).copied();
        functions.push(function_change(
            old,
            new,
            FunctionChangeKind::SignatureChanged,
            old_file.as_ref(),
            new_file.as_ref(),
        )?);
    }
    functions.sort_by(|left, right| (&left.name, left.change).cmp(&(&right.name, right.change)));

    let mut facts = Vec::new();
    if let Some((source, parsed, path)) = old_file
        .as_ref()
        .zip(old_parse.as_ref())
        .zip(old_path)
        .map(|((source, parsed), path)| (source, parsed, path))
    {
        collect_changed_syntax_facts(
            source,
            parsed.tree().root_node(),
            &old_exec_ranges,
            &old_unsafe_ranges,
            &old_functions,
            changed.kind,
            DiffSide::Old,
            path,
            &mut facts,
        )?;
    }
    if let Some((source, parsed, path)) = new_file
        .as_ref()
        .zip(new_parse.as_ref())
        .zip(new_path)
        .map(|((source, parsed), path)| (source, parsed, path))
    {
        collect_changed_syntax_facts(
            source,
            parsed.tree().root_node(),
            &new_exec_ranges,
            &new_unsafe_ranges,
            &new_functions,
            changed.kind,
            DiffSide::New,
            path,
            &mut facts,
        )?;
    }
    cancel_unchanged_syntax_facts(
        &mut facts,
        old_file.as_ref().zip(old_parse.as_ref()),
        new_file.as_ref().zip(new_parse.as_ref()),
    );

    for key in &added_keys {
        if let (Some(function), Some(source), Some(path)) =
            (new_function_map.get(key), new_file.as_ref(), new_path)
        {
            if !ranges_overlap_any(function.range, &new_unsafe_ranges) {
                facts.push(function_fact(
                    SemanticFactKind::FunctionAdded,
                    changed.kind,
                    DiffSide::New,
                    path,
                    source,
                    function,
                )?);
            }
        }
    }
    if primary_signature_changed {
        if let (Some(primary), Some(source), Some(path)) =
            (new_primary, new_file.as_ref(), new_path)
        {
            if !signature_changed_keys.contains(&primary.key)
                && !ranges_overlap_any(primary.signature_range, &new_unsafe_ranges)
            {
                facts.push(function_fact(
                    SemanticFactKind::FunctionSignatureChanged,
                    changed.kind,
                    DiffSide::New,
                    path,
                    source,
                    primary,
                )?);
            }
        }
    }
    for key in &deleted_keys {
        if let (Some(function), Some(source), Some(path)) =
            (old_function_map.get(key), old_file.as_ref(), old_path)
        {
            if !ranges_overlap_any(function.range, &old_unsafe_ranges) {
                facts.push(function_fact(
                    SemanticFactKind::FunctionDeleted,
                    changed.kind,
                    DiffSide::Old,
                    path,
                    source,
                    function,
                )?);
            }
        }
    }
    for key in &signature_changed_keys {
        if let (Some(function), Some(source), Some(path)) =
            (new_function_map.get(key), new_file.as_ref(), new_path)
        {
            if !ranges_overlap_any(function.signature_range, &new_unsafe_ranges) {
                facts.push(function_fact(
                    SemanticFactKind::FunctionSignatureChanged,
                    changed.kind,
                    DiffSide::New,
                    path,
                    source,
                    function,
                )?);
            }
        }
    }

    let inventory_member = old_path
        .map(normalize_path_string)
        .is_some_and(|path| public_api.contains(&path))
        || new_path
            .map(normalize_path_string)
            .is_some_and(|path| public_api.contains(&path));
    if inventory_member {
        if primary_signature_changed {
            if let (Some(primary), Some(source), Some(path)) =
                (new_primary, new_file.as_ref(), new_path)
            {
                let old_primary_unsafe = old_primary.is_some_and(|old_primary| {
                    ranges_overlap_any(old_primary.signature_range, &old_unsafe_ranges)
                });
                if !old_primary_unsafe
                    && !ranges_overlap_any(primary.signature_range, &new_unsafe_ranges)
                {
                    facts.push(function_fact(
                        SemanticFactKind::PublicApiSignatureChanged,
                        changed.kind,
                        DiffSide::New,
                        path,
                        source,
                        primary,
                    )?);
                }
            }
        }
    }

    if let (Some(source), Some(path)) = (old_file.as_ref(), old_path) {
        for index in &old_sections_touched {
            facts.push(section_fact(
                SemanticFactKind::SectionTouched,
                changed.kind,
                DiffSide::Old,
                path,
                source,
                &old_lines,
                &old_sections[*index],
            )?);
        }
    }
    if let (Some(source), Some(path)) = (new_file.as_ref(), new_path) {
        for index in &new_sections_touched {
            facts.push(section_fact(
                SemanticFactKind::SectionTouched,
                changed.kind,
                DiffSide::New,
                path,
                source,
                &new_lines,
                &new_sections[*index],
            )?);
        }
        for index in &added_section_indices {
            facts.push(section_fact(
                SemanticFactKind::SectionAdded,
                changed.kind,
                DiffSide::New,
                path,
                source,
                &new_lines,
                &new_sections[*index],
            )?);
        }
    }

    sort_and_dedup_facts(&mut facts);

    Ok(FileAnalysis {
        old_path: old_path.map(normalize_path_string),
        new_path: new_path.map(normalize_path_string),
        classification,
        parse_confidence,
        metrics,
        embedded_test_regions_touched,
        functions,
        facts,
    })
}

#[derive(Debug)]
struct FunctionInfo {
    key: String,
    name: String,
    range: ByteRange,
    signature_range: ByteRange,
    signature_fingerprint: Vec<String>,
    is_primary: bool,
}

fn collect_functions(
    source: &SourceFile,
    parsed: &ParseResult,
) -> Result<Vec<FunctionInfo>, DiffError> {
    let mut raw = Vec::new();
    let root = parsed.tree().root_node();
    collect_function_nodes(root, &mut raw);
    raw.sort_by_key(|node| node.start_byte());
    let primary_start = first_function_file_primary_start(root);

    let mut occurrences = BTreeMap::<(Vec<String>, String), usize>::new();
    let mut functions = Vec::new();
    for node in raw {
        let Some(name_node) = node.child_by_field_name("name") else {
            continue;
        };
        let name = source
            .slice(ByteRange::new(name_node.start_byte(), name_node.end_byte()))
            .map_err(DiffError::Position)?
            .to_owned();
        let owner_chain = function_owner_chain(node, source)?;
        let occurrence = occurrences
            .entry((owner_chain.clone(), name.clone()))
            .or_default();
        let owner = if owner_chain.is_empty() {
            "<file>".to_owned()
        } else {
            owner_chain.join("::")
        };
        let key = format!("{owner}::{name}#{occurrence}");
        *occurrence += 1;

        let signature_end = function_signature_end(node);
        let signature_range = ByteRange::new(node.start_byte(), signature_end);
        let signature_fingerprint =
            syntax_fingerprint(parsed.tree().root_node(), source.text(), signature_range);
        functions.push(FunctionInfo {
            key,
            name,
            range: ByteRange::new(node.start_byte(), node.end_byte()),
            signature_range,
            signature_fingerprint,
            is_primary: node.kind() == "function_definition"
                && node
                    .parent()
                    .is_some_and(|parent| parent.kind() == "source_file")
                && primary_start == Some(node.start_byte()),
        });
    }
    Ok(functions)
}

fn collect_function_nodes<'tree>(node: Node<'tree>, functions: &mut Vec<Node<'tree>>) {
    if node.kind() == "function_definition"
        || (node.kind() == "function_signature" && !has_function_definition_ancestor(node))
    {
        functions.push(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_function_nodes(child, functions);
    }
}

fn first_function_file_primary_start(root: Node<'_>) -> Option<usize> {
    let mut cursor = root.walk();
    root.named_children(&mut cursor)
        .find(|child| child.kind() != "comment")
        .filter(|child| child.kind() == "function_definition")
        .map(|child| child.start_byte())
}

#[allow(clippy::collapsible_if)]
fn function_owner_chain(mut node: Node<'_>, source: &SourceFile) -> Result<Vec<String>, DiffError> {
    let mut owners = Vec::new();
    while let Some(parent) = node.parent() {
        if matches!(parent.kind(), "function_definition" | "function_signature") {
            if let Some(name_node) = parent.child_by_field_name("name") {
                owners.push(
                    source
                        .slice(ByteRange::new(name_node.start_byte(), name_node.end_byte()))
                        .map_err(DiffError::Position)?
                        .to_owned(),
                );
            }
        }
        node = parent;
    }
    owners.reverse();
    Ok(owners)
}

fn has_function_definition_ancestor(mut node: Node<'_>) -> bool {
    while let Some(parent) = node.parent() {
        if parent.kind() == "function_definition" {
            return true;
        }
        node = parent;
    }
    false
}

fn collect_smallest_touched_functions(
    functions: &[FunctionInfo],
    changed_ranges: &[ByteRange],
    touched: &mut BTreeSet<String>,
) {
    for changed in changed_ranges {
        if let Some(function) = functions
            .iter()
            .filter(|function| range_contains(function.range, *changed))
            .min_by_key(|function| function.range.end - function.range.start)
        {
            touched.insert(function.key.clone());
        }
    }
}

fn primary_function(functions: &[FunctionInfo]) -> Option<&FunctionInfo> {
    functions.iter().find(|function| function.is_primary)
}

fn range_contains(container: ByteRange, inner: ByteRange) -> bool {
    container.start <= inner.start && inner.end <= container.end
}

fn function_signature_end(node: Node<'_>) -> usize {
    let mut end = node.start_byte();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "block" | "arguments_statement" | "comment" => break,
            _ => end = end.max(child.end_byte()),
        }
    }
    end.max(node.start_byte())
}

fn function_change(
    old: Option<&FunctionInfo>,
    new: Option<&FunctionInfo>,
    change: FunctionChangeKind,
    old_source: Option<&SourceFile>,
    new_source: Option<&SourceFile>,
) -> Result<FunctionChange, DiffError> {
    let name = new
        .or(old)
        .map(|function| function.name.clone())
        .unwrap_or_default();
    Ok(FunctionChange {
        name,
        change,
        old_range: old
            .zip(old_source)
            .map(|(function, source)| source_range(source, function.range))
            .transpose()?,
        new_range: new
            .zip(new_source)
            .map(|(function, source)| source_range(source, function.range))
            .transpose()?,
    })
}

fn function_fact(
    kind: SemanticFactKind,
    change: ChangeKind,
    side: DiffSide,
    path: &Path,
    source: &SourceFile,
    function: &FunctionInfo,
) -> Result<SemanticFact, DiffError> {
    Ok(SemanticFact {
        kind,
        change,
        side,
        path: normalize_path_string(path),
        range: source_range(source, function.signature_range)?,
        enclosing_function: Some(function.name.clone()),
        identifiers: vec![function.name.clone()],
        call_or_index_targets: Vec::new(),
        field_paths: Vec::new(),
        operators: Vec::new(),
        literals: Vec::new(),
    })
}

#[allow(clippy::collapsible_if, clippy::too_many_arguments)]
fn collect_changed_syntax_facts(
    source: &SourceFile,
    node: Node<'_>,
    changed_ranges: &[ByteRange],
    unsafe_ranges: &[ByteRange],
    functions: &[FunctionInfo],
    change: ChangeKind,
    side: DiffSide,
    path: &Path,
    facts: &mut Vec<SemanticFact>,
) -> Result<(), DiffError> {
    let range = ByteRange::new(node.start_byte(), node.end_byte());
    if !ranges_overlap_any(range, changed_ranges) {
        return Ok(());
    }

    if let Some(kind) = syntax_fact_kind(node.kind()) {
        let relevant_range = if kind == SemanticFactKind::ControlFlowChanged {
            control_flow_header_range(node)
        } else {
            range
        };
        if !ranges_overlap_any(range, unsafe_ranges) {
            if ranges_overlap_any(relevant_range, changed_ranges) {
                facts.push(make_syntax_fact(
                    kind,
                    change,
                    side,
                    path,
                    source,
                    node,
                    relevant_range,
                    functions,
                )?);
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_changed_syntax_facts(
            source,
            child,
            changed_ranges,
            unsafe_ranges,
            functions,
            change,
            side,
            path,
            facts,
        )?;
    }
    Ok(())
}

fn syntax_fact_kind(kind: &str) -> Option<SemanticFactKind> {
    match kind {
        "persistent_operator" => Some(SemanticFactKind::PersistentChanged),
        "global_operator" => Some(SemanticFactKind::GlobalChanged),
        "assignment" => Some(SemanticFactKind::AssignmentChanged),
        "number"
        | "binary_operator"
        | "unary_operator"
        | "boolean_operator"
        | "comparison_operator"
        | "range" => Some(SemanticFactKind::NumericExpressionChanged),
        "function_call" => Some(SemanticFactKind::CallOrIndexChanged),
        "if_statement" | "elseif_clause" | "for_statement" | "while_statement"
        | "switch_statement" | "case_clause" | "otherwise_clause" | "try_statement"
        | "catch_clause" | "spmd_statement" => Some(SemanticFactKind::ControlFlowChanged),
        _ => None,
    }
}

fn control_flow_header_range(node: Node<'_>) -> ByteRange {
    let mut end = node.end_byte();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "block" {
            end = child.start_byte();
            break;
        }
    }
    ByteRange::new(node.start_byte(), end)
}

#[allow(clippy::too_many_arguments)]
fn make_syntax_fact(
    kind: SemanticFactKind,
    change: ChangeKind,
    side: DiffSide,
    path: &Path,
    source: &SourceFile,
    node: Node<'_>,
    context_range: ByteRange,
    functions: &[FunctionInfo],
) -> Result<SemanticFact, DiffError> {
    let range = context_range;
    let mut identifiers = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut field_paths = BTreeSet::new();
    let mut operators = BTreeSet::new();
    let mut literals = BTreeSet::new();
    collect_context(
        node,
        source,
        context_range,
        &mut identifiers,
        &mut targets,
        &mut field_paths,
        &mut operators,
        &mut literals,
    )?;

    let enclosing_function = functions
        .iter()
        .filter(|function| function.range.start <= range.start && range.end <= function.range.end)
        .min_by_key(|function| function.range.end - function.range.start)
        .map(|function| function.name.clone());

    Ok(SemanticFact {
        kind,
        change,
        side,
        path: normalize_path_string(path),
        range: source_range(source, range)?,
        enclosing_function,
        identifiers: identifiers.into_iter().collect(),
        call_or_index_targets: targets.into_iter().collect(),
        field_paths: field_paths.into_iter().collect(),
        operators: operators.into_iter().collect(),
        literals: literals.into_iter().collect(),
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_context(
    node: Node<'_>,
    source: &SourceFile,
    context_range: ByteRange,
    identifiers: &mut BTreeSet<String>,
    targets: &mut BTreeSet<String>,
    field_paths: &mut BTreeSet<String>,
    operators: &mut BTreeSet<String>,
    literals: &mut BTreeSet<String>,
) -> Result<(), DiffError> {
    let range = ByteRange::new(node.start_byte(), node.end_byte());
    if !range_overlaps_or_touches(range, context_range) {
        return Ok(());
    }
    match node.kind() {
        "identifier" => {
            identifiers.insert(source.slice(range).map_err(DiffError::Position)?.to_owned());
        }
        "number" | "string" => {
            literals.insert(source.slice(range).map_err(DiffError::Position)?.to_owned());
        }
        "field_expression" => {
            let value = source.slice(range).map_err(DiffError::Position)?;
            field_paths.insert(
                value
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect(),
            );
        }
        "function_call" => {
            if let Some(name) = node.child_by_field_name("name") {
                let value = source
                    .slice(ByteRange::new(name.start_byte(), name.end_byte()))
                    .map_err(DiffError::Position)?;
                targets.insert(
                    value
                        .chars()
                        .filter(|character| !character.is_whitespace())
                        .collect(),
                );
            }
        }
        _ => {}
    }

    if node.child_count() == 0 && !node.is_named() {
        let token = source.slice(range).map_err(DiffError::Position)?;
        if is_operator_token(token) {
            operators.insert(token.to_owned());
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_context(
            child,
            source,
            context_range,
            identifiers,
            targets,
            field_paths,
            operators,
            literals,
        )?;
    }
    Ok(())
}

fn cancel_unchanged_syntax_facts(
    facts: &mut Vec<SemanticFact>,
    old: Option<(&SourceFile, &ParseResult)>,
    new: Option<(&SourceFile, &ParseResult)>,
) {
    let mut old_counts = BTreeMap::<SemanticFactIdentity, usize>::new();
    let mut new_counts = BTreeMap::<SemanticFactIdentity, usize>::new();
    for fact in facts.iter() {
        let Some(identity) = semantic_fact_identity(fact, old, new) else {
            continue;
        };
        match fact.side {
            DiffSide::Old => *old_counts.entry(identity).or_default() += 1,
            DiffSide::New => *new_counts.entry(identity).or_default() += 1,
        }
    }

    let mut pairs = BTreeMap::<SemanticFactIdentity, usize>::new();
    for (identity, old_count) in old_counts {
        let pair_count = old_count.min(new_counts.get(&identity).copied().unwrap_or(0));
        if pair_count > 0 {
            pairs.insert(identity, pair_count);
        }
    }

    let mut removed_old = BTreeMap::<SemanticFactIdentity, usize>::new();
    let mut removed_new = BTreeMap::<SemanticFactIdentity, usize>::new();
    facts.retain(|fact| {
        let Some(identity) = semantic_fact_identity(fact, old, new) else {
            return true;
        };
        let Some(limit) = pairs.get(&identity).copied() else {
            return true;
        };
        let removed = match fact.side {
            DiffSide::Old => removed_old.entry(identity).or_default(),
            DiffSide::New => removed_new.entry(identity).or_default(),
        };
        if *removed < limit {
            *removed += 1;
            false
        } else {
            true
        }
    });
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SemanticFactIdentity {
    kind: SemanticFactKind,
    enclosing_function: Option<String>,
    identifiers: Vec<String>,
    call_or_index_targets: Vec<String>,
    field_paths: Vec<String>,
    operators: Vec<String>,
    literals: Vec<String>,
    syntax_fingerprint: Vec<String>,
}

fn semantic_fact_identity(
    fact: &SemanticFact,
    old: Option<(&SourceFile, &ParseResult)>,
    new: Option<(&SourceFile, &ParseResult)>,
) -> Option<SemanticFactIdentity> {
    let (source, parsed) = match fact.side {
        DiffSide::Old => old?,
        DiffSide::New => new?,
    };
    let range = ByteRange::new(fact.range.start_byte, fact.range.end_byte);
    Some(SemanticFactIdentity {
        kind: fact.kind,
        enclosing_function: fact.enclosing_function.clone(),
        identifiers: fact.identifiers.clone(),
        call_or_index_targets: fact.call_or_index_targets.clone(),
        field_paths: fact.field_paths.clone(),
        operators: fact.operators.clone(),
        literals: fact.literals.clone(),
        syntax_fingerprint: syntax_fingerprint(parsed.tree().root_node(), source.text(), range),
    })
}

fn is_operator_token(token: &str) -> bool {
    matches!(
        token,
        "+" | "-"
            | "*"
            | "/"
            | "\\"
            | "^"
            | ".*"
            | "./"
            | ".\\"
            | ".^"
            | "=="
            | "~="
            | "<"
            | "<="
            | ">"
            | ">="
            | "&"
            | "|"
            | "&&"
            | "||"
            | "~"
            | ":"
            | "="
    )
}

fn count_control_flow_in_ranges(node: Node<'_>, changed_ranges: &[ByteRange]) -> usize {
    let own = usize::from(
        matches!(
            node.kind(),
            "if_statement"
                | "elseif_clause"
                | "for_statement"
                | "while_statement"
                | "case_clause"
                | "catch_clause"
                | "spmd_statement"
        ) && ranges_overlap_any(control_flow_header_range(node), changed_ranges),
    );
    let mut count = own;
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        count += count_control_flow_in_ranges(child, changed_ranges);
    }
    count
}

#[derive(Debug, Clone)]
struct LineInfo {
    start: usize,
    end: usize,
    text: String,
}

fn line_table(source: &str) -> Vec<LineInfo> {
    if source.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let end = source[start..]
            .find('\n')
            .map_or(source.len(), |offset| start + offset + 1);
        let content_end = if end > start && source.as_bytes()[end - 1] == b'\n' {
            end - 1
        } else {
            end
        };
        let content_end =
            if content_end > start && source.as_bytes().get(content_end - 1) == Some(&b'\r') {
                content_end - 1
            } else {
                content_end
            };
        lines.push(LineInfo {
            start,
            end,
            text: source[start..content_end].to_owned(),
        });
        start = end;
    }
    lines
}

#[derive(Debug, Clone, Copy)]
struct DiffChunk {
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
}

fn formatting_equivalence_chunks(chunk: DiffChunk) -> Vec<DiffChunk> {
    let old_len = chunk.old_end - chunk.old_start;
    let new_len = chunk.new_end - chunk.new_start;
    if old_len == new_len && old_len > 1 {
        return (0..old_len)
            .map(|offset| DiffChunk {
                old_start: chunk.old_start + offset,
                old_end: chunk.old_start + offset + 1,
                new_start: chunk.new_start + offset,
                new_end: chunk.new_start + offset + 1,
            })
            .collect();
    }
    vec![chunk]
}

fn diff_chunks(old: &[LineInfo], new: &[LineInfo]) -> Vec<DiffChunk> {
    let mut chunks = Vec::new();
    diff_chunks_range(old, 0, old.len(), new, 0, new.len(), &mut chunks);
    chunks
}

fn diff_chunks_range(
    old: &[LineInfo],
    mut old_start: usize,
    mut old_end: usize,
    new: &[LineInfo],
    mut new_start: usize,
    mut new_end: usize,
    chunks: &mut Vec<DiffChunk>,
) {
    while old_start < old_end && new_start < new_end && old[old_start].text == new[new_start].text {
        old_start += 1;
        new_start += 1;
    }
    while old_end > old_start
        && new_end > new_start
        && old[old_end - 1].text == new[new_end - 1].text
    {
        old_end -= 1;
        new_end -= 1;
    }

    let old_len = old_end - old_start;
    let new_len = new_end - new_start;
    if old_len == 0 && new_len == 0 {
        return;
    }
    if old_len == 0 || new_len == 0 {
        chunks.push(DiffChunk {
            old_start,
            old_end,
            new_start,
            new_end,
        });
        return;
    }

    if old_len.saturating_mul(new_len) > LCS_CELL_LIMIT {
        if let Some((old_anchor, new_anchor)) =
            find_common_anchor(old, old_start, old_end, new, new_start, new_end)
        {
            diff_chunks_range(
                old, old_start, old_anchor, new, new_start, new_anchor, chunks,
            );
            diff_chunks_range(
                old,
                old_anchor + 1,
                old_end,
                new,
                new_anchor + 1,
                new_end,
                chunks,
            );
        } else {
            chunks.push(DiffChunk {
                old_start,
                old_end,
                new_start,
                new_end,
            });
        }
        return;
    }

    let width = new_len + 1;
    let mut lcs = vec![0_u32; (old_len + 1) * (new_len + 1)];
    for old_index in (0..old_len).rev() {
        for new_index in (0..new_len).rev() {
            let slot = old_index * width + new_index;
            lcs[slot] = if old[old_start + old_index].text == new[new_start + new_index].text {
                1 + lcs[(old_index + 1) * width + new_index + 1]
            } else {
                lcs[(old_index + 1) * width + new_index].max(lcs[old_index * width + new_index + 1])
            };
        }
    }

    let mut old_index = 0;
    let mut new_index = 0;
    let mut chunk_start: Option<(usize, usize)> = None;

    let flush = |chunks: &mut Vec<DiffChunk>,
                 chunk_start: &mut Option<(usize, usize)>,
                 old_index: usize,
                 new_index: usize| {
        if let Some((chunk_old_start, chunk_new_start)) = chunk_start.take() {
            chunks.push(DiffChunk {
                old_start: old_start + chunk_old_start,
                old_end: old_start + old_index,
                new_start: new_start + chunk_new_start,
                new_end: new_start + new_index,
            });
        }
    };

    while old_index < old_len && new_index < new_len {
        if old[old_start + old_index].text == new[new_start + new_index].text {
            flush(chunks, &mut chunk_start, old_index, new_index);
            old_index += 1;
            new_index += 1;
        } else {
            chunk_start.get_or_insert((old_index, new_index));
            let delete_score = lcs[(old_index + 1) * width + new_index];
            let add_score = lcs[old_index * width + new_index + 1];
            if delete_score >= add_score {
                old_index += 1;
            } else {
                new_index += 1;
            }
        }
    }
    if old_index < old_len || new_index < new_len {
        chunk_start.get_or_insert((old_index, new_index));
        old_index = old_len;
        new_index = new_len;
    }
    flush(chunks, &mut chunk_start, old_index, new_index);
}

fn find_common_anchor(
    old: &[LineInfo],
    old_start: usize,
    old_end: usize,
    new: &[LineInfo],
    new_start: usize,
    new_end: usize,
) -> Option<(usize, usize)> {
    let old_mid = old_start + (old_end - old_start) / 2;
    let new_mid = new_start + (new_end - new_start) / 2;
    let mut old_lines = BTreeMap::<&str, (usize, usize)>::new();
    let mut new_lines = BTreeMap::<&str, (usize, usize)>::new();

    for (index, line) in old.iter().enumerate().take(old_end).skip(old_start) {
        let entry = old_lines.entry(&line.text).or_insert((0, index));
        entry.0 += 1;
        if index.abs_diff(old_mid) < entry.1.abs_diff(old_mid) {
            entry.1 = index;
        }
    }
    for (index, line) in new.iter().enumerate().take(new_end).skip(new_start) {
        let entry = new_lines.entry(&line.text).or_insert((0, index));
        entry.0 += 1;
        if index.abs_diff(new_mid) < entry.1.abs_diff(new_mid) {
            entry.1 = index;
        }
    }

    let old_len = old_end - old_start;
    let new_len = new_end - new_start;
    old_lines
        .iter()
        .filter_map(|(text, (old_count, old_index))| {
            let (new_count, new_index) = new_lines.get(*text)?;
            let frequency = (*old_count).saturating_mul(*new_count);
            let relative_distance = (*old_index - old_start)
                .saturating_mul(new_len)
                .abs_diff((*new_index - new_start).saturating_mul(old_len));
            let midpoint_distance = (*old_index).abs_diff(old_mid) + (*new_index).abs_diff(new_mid);
            Some((
                (
                    frequency,
                    relative_distance,
                    midpoint_distance,
                    *old_index,
                    *new_index,
                ),
                (*old_index, *new_index),
            ))
        })
        .min_by_key(|(score, _)| *score)
        .map(|(_, anchor)| anchor)
}

fn executable_line_flags(lines: &[LineInfo]) -> Vec<bool> {
    let mut flags = Vec::with_capacity(lines.len());
    let mut in_block_comment = false;
    for line in lines {
        let trimmed = line.text.trim();
        if in_block_comment {
            if trimmed == "%}" {
                in_block_comment = false;
            }
            flags.push(false);
            continue;
        }
        if trimmed == "%{" {
            in_block_comment = true;
            flags.push(false);
            continue;
        }
        flags.push(!trimmed.is_empty() && !trimmed.starts_with('%'));
    }
    flags
}

#[allow(clippy::too_many_arguments)]
fn replacement_is_syntax_equivalent(
    old_source: &SourceFile,
    old_parsed: &ParseResult,
    old_lines: &[LineInfo],
    old_start: usize,
    old_end: usize,
    new_source: &SourceFile,
    new_parsed: &ParseResult,
    new_lines: &[LineInfo],
    new_start: usize,
    new_end: usize,
) -> bool {
    if old_start == old_end || new_start == new_end {
        return false;
    }
    let old_range = line_slice_range(old_lines, old_start, old_end, old_source.len());
    let new_range = line_slice_range(new_lines, new_start, new_end, new_source.len());
    if diagnostics_overlap_ranges(old_parsed, &[old_range])
        || diagnostics_overlap_ranges(new_parsed, &[new_range])
    {
        return false;
    }
    if intersects_semantic_preservation_zone(old_source, old_parsed, old_range)
        || intersects_semantic_preservation_zone(new_source, new_parsed, new_range)
    {
        return false;
    }
    syntax_fingerprint(old_parsed.tree().root_node(), old_source.text(), old_range)
        == syntax_fingerprint(new_parsed.tree().root_node(), new_source.text(), new_range)
}

fn intersects_semantic_preservation_zone(
    source: &SourceFile,
    parsed: &ParseResult,
    range: ByteRange,
) -> bool {
    collect_preservation_zones(parsed.tree().root_node(), source.text())
        .into_iter()
        .filter(|zone| !matches!(zone.reason, PreservationReason::Comment))
        .any(|zone| range_overlaps_or_touches(range, zone.range))
}

fn syntax_fingerprint(root: Node<'_>, source: &str, range: ByteRange) -> Vec<String> {
    let mut fingerprint = Vec::new();
    collect_syntax_fingerprint(root, source, range, &mut fingerprint);
    fingerprint
}

fn collect_syntax_fingerprint(
    node: Node<'_>,
    source: &str,
    range: ByteRange,
    fingerprint: &mut Vec<String>,
) {
    let node_range = ByteRange::new(node.start_byte(), node.end_byte());
    if !range_overlaps_or_touches(node_range, range) || node.kind() == "comment" {
        return;
    }

    fingerprint.push(format!("({}", node.kind()));
    if node.child_count() == 0 {
        if let Some(text) = source.get(node.start_byte()..node.end_byte()) {
            fingerprint.push(text.to_owned());
        }
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_syntax_fingerprint(child, source, range, fingerprint);
        }
    }
    fingerprint.push(")".to_owned());
}

fn line_slice_range(lines: &[LineInfo], start: usize, end: usize, source_len: usize) -> ByteRange {
    if start == end {
        let point = lines.get(start).map_or(source_len, |line| line.start);
        return ByteRange::new(point, point);
    }
    ByteRange::new(lines[start].start, lines[end - 1].end)
}

fn line_indices_to_ranges(
    lines: &[LineInfo],
    indices: &BTreeSet<usize>,
    source_len: usize,
) -> Vec<ByteRange> {
    let mut ranges = Vec::new();
    let mut run_start: Option<usize> = None;
    let mut previous: Option<usize> = None;

    for index in indices {
        if previous.is_some_and(|value| *index != value + 1) {
            if let (Some(start), Some(end)) = (run_start.take(), previous) {
                ranges.push(line_slice_range(lines, start, end + 1, source_len));
            }
            run_start = Some(*index);
        } else {
            run_start.get_or_insert(*index);
        }
        previous = Some(*index);
    }
    if let (Some(start), Some(end)) = (run_start, previous) {
        ranges.push(line_slice_range(lines, start, end + 1, source_len));
    }
    ranges
}

fn line_indices_to_single_line_ranges(
    lines: &[LineInfo],
    indices: &BTreeSet<usize>,
    source_len: usize,
) -> Vec<ByteRange> {
    indices
        .iter()
        .map(|index| line_slice_range(lines, *index, *index + 1, source_len))
        .collect()
}

fn diagnostics_overlap_ranges(parsed: &ParseResult, ranges: &[ByteRange]) -> bool {
    parsed.diagnostics().iter().any(|diagnostic| {
        ranges
            .iter()
            .any(|range| range_overlaps_or_touches(diagnostic.range, *range))
    })
}

fn recovery_ranges(parsed: &ParseResult, changed_ranges: &[ByteRange]) -> Vec<ByteRange> {
    parsed
        .diagnostics()
        .iter()
        .filter(|diagnostic| {
            changed_ranges
                .iter()
                .any(|range| range_overlaps_or_touches(diagnostic.range, *range))
        })
        .map(|diagnostic| diagnostic.range)
        .collect()
}

fn range_overlaps_or_touches(left: ByteRange, right: ByteRange) -> bool {
    left.overlaps(right)
        || (left.is_empty() && right.touches_point(left.start))
        || (right.is_empty() && left.touches_point(right.start))
}

fn ranges_overlap_any(range: ByteRange, others: &[ByteRange]) -> bool {
    others
        .iter()
        .any(|other| range_overlaps_or_touches(range, *other))
}

#[derive(Debug, Clone)]
struct SectionInfo {
    header_line: usize,
    start_line: usize,
    end_line: usize,
}

fn collect_sections(lines: &[LineInfo]) -> Vec<SectionInfo> {
    let visible = marker_visibility(lines);
    let headers = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (visible[index] && line.text.trim_start().starts_with("%%")).then_some(index)
        })
        .collect::<Vec<_>>();
    headers
        .iter()
        .enumerate()
        .map(|(position, header)| SectionInfo {
            header_line: *header,
            start_line: *header,
            end_line: headers.get(position + 1).copied().unwrap_or(lines.len()),
        })
        .collect()
}

fn collect_embedded_test_regions(lines: &[LineInfo]) -> Vec<SectionInfo> {
    let visible = marker_visibility(lines);
    let mut regions = Vec::new();
    let mut start = None;
    for (index, line) in lines.iter().enumerate() {
        if visible[index] && line.text.trim_start().starts_with("%!") {
            start.get_or_insert(index);
        } else if let Some(region_start) = start.take() {
            regions.push(SectionInfo {
                header_line: region_start,
                start_line: region_start,
                end_line: index,
            });
        }
    }
    if let Some(region_start) = start {
        regions.push(SectionInfo {
            header_line: region_start,
            start_line: region_start,
            end_line: lines.len(),
        });
    }
    regions
}

fn marker_visibility(lines: &[LineInfo]) -> Vec<bool> {
    let mut visible = Vec::with_capacity(lines.len());
    let mut in_block_comment = false;
    for line in lines {
        let trimmed = line.text.trim();
        if in_block_comment {
            visible.push(false);
            if trimmed == "%}" {
                in_block_comment = false;
            }
        } else if trimmed == "%{" {
            visible.push(false);
            in_block_comment = true;
        } else {
            visible.push(true);
        }
    }
    visible
}

fn sections_touched(sections: &[SectionInfo], changed_lines: &BTreeSet<usize>) -> BTreeSet<usize> {
    sections
        .iter()
        .enumerate()
        .filter_map(|(index, section)| {
            changed_lines
                .range(section.start_line..section.end_line)
                .next()
                .map(|_| index)
        })
        .collect()
}

fn added_sections_by_chunks(
    chunks: &[DiffChunk],
    old_sections: &[SectionInfo],
    new_sections: &[SectionInfo],
) -> BTreeSet<usize> {
    let mut added = BTreeSet::new();
    for chunk in chunks {
        let old_count = old_sections
            .iter()
            .filter(|section| {
                chunk.old_start <= section.header_line && section.header_line < chunk.old_end
            })
            .count();
        let new_indices = new_sections
            .iter()
            .enumerate()
            .filter_map(|(index, section)| {
                (chunk.new_start <= section.header_line && section.header_line < chunk.new_end)
                    .then_some(index)
            })
            .collect::<Vec<_>>();
        for index in new_indices.into_iter().skip(old_count) {
            added.insert(index);
        }
    }
    added
}

fn section_fact(
    kind: SemanticFactKind,
    change: ChangeKind,
    side: DiffSide,
    path: &Path,
    source: &SourceFile,
    lines: &[LineInfo],
    section: &SectionInfo,
) -> Result<SemanticFact, DiffError> {
    let range = line_slice_range(
        lines,
        section.header_line,
        section.header_line + 1,
        source.len(),
    );
    Ok(SemanticFact {
        kind,
        change,
        side,
        path: normalize_path_string(path),
        range: source_range(source, range)?,
        enclosing_function: None,
        identifiers: Vec::new(),
        call_or_index_targets: Vec::new(),
        field_paths: Vec::new(),
        operators: Vec::new(),
        literals: Vec::new(),
    })
}

fn source_range(source: &SourceFile, range: ByteRange) -> Result<SourceRange, DiffError> {
    Ok(SourceRange {
        start: source.position(range.start).map_err(DiffError::Position)?,
        end: source.position(range.end).map_err(DiffError::Position)?,
        start_byte: range.start,
        end_byte: range.end,
    })
}

fn sort_and_dedup_facts(facts: &mut Vec<SemanticFact>) {
    facts.sort_by(|left, right| {
        (
            &left.path,
            left.side,
            left.range.start_byte,
            left.range.end_byte,
            left.kind,
            &left.enclosing_function,
            &left.identifiers,
            &left.call_or_index_targets,
            &left.field_paths,
            &left.operators,
            &left.literals,
        )
            .cmp(&(
                &right.path,
                right.side,
                right.range.start_byte,
                right.range.end_byte,
                right.kind,
                &right.enclosing_function,
                &right.identifiers,
                &right.call_or_index_targets,
                &right.field_paths,
                &right.operators,
                &right.literals,
            ))
    });
    facts.dedup();
}

fn evaluate_thresholds(
    scope: &str,
    metrics: DiffMetrics,
    thresholds: &DiffMetricThresholds,
    decision: &mut DiffDecision,
    triggers: &mut Vec<PolicyTrigger>,
) {
    for (name, value, threshold) in [
        (
            "added_lines",
            metrics.added_executable_lines,
            thresholds.added_lines.as_ref(),
        ),
        (
            "deleted_lines",
            metrics.deleted_executable_lines,
            thresholds.deleted_lines.as_ref(),
        ),
        (
            "files_touched",
            metrics.files_touched,
            thresholds.files_touched.as_ref(),
        ),
        (
            "functions_touched",
            metrics.functions_touched,
            thresholds.functions_touched.as_ref(),
        ),
        (
            "functions_added",
            metrics.functions_added,
            thresholds.functions_added.as_ref(),
        ),
        (
            "control_flow",
            metrics.control_flow_increase,
            thresholds.control_flow.as_ref(),
        ),
        (
            "sections_touched",
            metrics.sections_touched,
            thresholds.sections_touched.as_ref(),
        ),
        (
            "sections_added",
            metrics.sections_added,
            thresholds.sections_added.as_ref(),
        ),
    ] {
        if let Some(threshold) = threshold {
            evaluate_threshold(scope, name, value, threshold, decision, triggers);
        }
    }
}

fn evaluate_threshold(
    scope: &str,
    metric: &str,
    value: usize,
    threshold: &DiffThreshold,
    decision: &mut DiffDecision,
    triggers: &mut Vec<PolicyTrigger>,
) {
    let (severity, boundary) = if value >= threshold.reject {
        (DiffDecision::RejectSplit, threshold.reject)
    } else if value >= threshold.review {
        (DiffDecision::Review, threshold.review)
    } else {
        return;
    };
    raise_decision(decision, severity);
    triggers.push(PolicyTrigger {
        id: format!(
            "THRESHOLD.{}.{}.{}",
            scope,
            metric,
            match severity {
                DiffDecision::Review => "REVIEW",
                DiffDecision::RejectSplit => "REJECT_SPLIT",
                DiffDecision::Accept => "ACCEPT",
            }
        ),
        severity,
        scope: scope.to_owned(),
        metric: Some(metric.to_owned()),
        value: Some(value),
        threshold: Some(boundary),
    });
}

fn evaluate_risks(
    root: &Path,
    files: &[FileAnalysis],
    risks: &[DiffRiskConfig],
    decision: &mut DiffDecision,
    triggers: &mut Vec<PolicyTrigger>,
) -> Result<(), DiffError> {
    let mut matched = BTreeMap::<String, DiffDecision>::new();
    for risk in risks {
        let path_matcher =
            PathMatcher::new(root, &risk.paths, &format!("diff.risk {:?}", risk.id))?;
        let kinds = risk_fact_kinds(risk);
        let is_match = files.iter().flat_map(|file| &file.facts).any(|fact| {
            kinds.contains(&fact.kind)
                && (risk.paths.is_empty() || path_matcher.matches(Path::new(&fact.path)))
                && risk_context_matches(risk, fact)
        });
        if !is_match {
            continue;
        }
        let severity = match risk.severity {
            DiffRiskSeverity::Review => DiffDecision::Review,
            DiffRiskSeverity::Reject => DiffDecision::RejectSplit,
        };
        matched
            .entry(risk.id.clone())
            .and_modify(|current| raise_decision(current, severity))
            .or_insert(severity);
    }

    for (id, severity) in matched {
        raise_decision(decision, severity);
        triggers.push(PolicyTrigger {
            id,
            severity,
            scope: "risk".to_owned(),
            metric: None,
            value: None,
            threshold: None,
        });
    }
    Ok(())
}

fn risk_fact_kinds(risk: &DiffRiskConfig) -> BTreeSet<SemanticFactKind> {
    risk.fact
        .iter()
        .copied()
        .chain(risk.facts.iter().copied())
        .map(Into::into)
        .collect()
}

fn risk_context_matches(risk: &DiffRiskConfig, fact: &SemanticFact) -> bool {
    if risk.match_mode == DiffRiskMatch::Any {
        return true;
    }
    selector_matches(&risk.identifiers, &fact.identifiers)
        && selector_matches(&risk.targets, &fact.call_or_index_targets)
        && selector_matches(&risk.field_paths, &fact.field_paths)
        && selector_matches(
            &risk.functions,
            fact.enclosing_function.as_slice(),
        )
        && selector_matches(&risk.operators, &fact.operators)
        && selector_matches(&risk.literals, &fact.literals)
}

fn selector_matches(configured: &[String], actual: &[String]) -> bool {
    configured.is_empty()
        || configured
            .iter()
            .any(|configured| actual.iter().any(|actual| actual == configured))
}

fn raise_decision(current: &mut DiffDecision, candidate: DiffDecision) {
    if candidate > *current {
        *current = candidate;
    }
}

struct PathMatcher {
    root: PathBuf,
    matcher: Option<Gitignore>,
}

impl PathMatcher {
    fn new(root: &Path, patterns: &[String], context: &str) -> Result<Self, DiffError> {
        if patterns.is_empty() {
            return Ok(Self {
                root: root.to_path_buf(),
                matcher: None,
            });
        }
        let mut builder = GitignoreBuilder::new(root);
        for pattern in patterns {
            builder
                .add_line(None, pattern)
                .map_err(|error| DiffError::Pattern {
                    context: context.to_owned(),
                    pattern: pattern.clone(),
                    message: error.to_string(),
                })?;
        }
        let matcher = builder.build().map_err(|error| DiffError::Pattern {
            context: context.to_owned(),
            pattern: "<combined>".to_owned(),
            message: error.to_string(),
        })?;
        Ok(Self {
            root: root.to_path_buf(),
            matcher: Some(matcher),
        })
    }

    fn matches(&self, relative: &Path) -> bool {
        self.matcher.as_ref().is_some_and(|matcher| {
            matcher
                .matched_path_or_any_parents(self.root.join(relative), false)
                .is_ignore()
        })
    }
}

fn selected_path_matches(changed: &ChangedFile, matcher: &PathMatcher) -> bool {
    changed
        .new_path
        .as_deref()
        .or(changed.old_path.as_deref())
        .is_some_and(|path| matcher.matches(path))
}

fn changed_has_matlab_path(changed: &ChangedFile) -> bool {
    changed.old_path.as_deref().is_some_and(is_matlab_path)
        || changed.new_path.as_deref().is_some_and(is_matlab_path)
}

fn is_matlab_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("m"))
}

fn file_analysis_matches(file: &FileAnalysis, matcher: &PathMatcher) -> bool {
    file.old_path
        .as_deref()
        .is_some_and(|path| matcher.matches(Path::new(path)))
        || file
            .new_path
            .as_deref()
            .is_some_and(|path| matcher.matches(Path::new(path)))
}

fn load_public_api_inventory(
    root: &Path,
    config: &DiffConfig,
) -> Result<BTreeSet<String>, DiffError> {
    let Some(inventory) = config.public_api_inventory.as_ref() else {
        return Ok(BTreeSet::new());
    };
    let inventory = normalize_relative_path(inventory)?;
    let inventory_path = root.join(inventory);
    let contents = fs::read_to_string(&inventory_path)
        .map_err(|error| DiffError::Io(inventory_path.clone(), error))?;
    let api_root = normalize_relative_path(&config.public_api_root)?;
    let mut entries = BTreeSet::new();
    for line in contents.lines() {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        let entry = normalize_relative_path(Path::new(entry))?;
        entries.insert(normalize_path_string(&api_root.join(entry)));
    }
    Ok(entries)
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, DiffError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(DiffError::InvalidConfigPath(path.to_path_buf()));
                }
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(DiffError::InvalidConfigPath(path.to_path_buf()));
            }
        }
    }
    Ok(normalized)
}

fn normalize_path_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::CurDir => None,
            Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            Component::ParentDir => Some("..".to_owned()),
            Component::RootDir => None,
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().into_owned()),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn display_changed_path(changed: &ChangedFile) -> String {
    changed
        .new_path
        .as_deref()
        .or(changed.old_path.as_deref())
        .map(normalize_path_string)
        .unwrap_or_else(|| "<unknown>".to_owned())
}

#[derive(Debug)]
pub enum DiffError {
    Parser(ParserError),
    Position(PositionError),
    Io(PathBuf, std::io::Error),
    Pattern {
        context: String,
        pattern: String,
        message: String,
    },
    InvalidConfigPath(PathBuf),
}

impl fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parser(error) => write!(formatter, "{error}"),
            Self::Position(error) => write!(formatter, "{error}"),
            Self::Io(path, error) => write!(formatter, "{}: {error}", path.display()),
            Self::Pattern {
                context,
                pattern,
                message,
            } => write!(
                formatter,
                "{context} contains invalid path pattern {pattern:?}: {message}"
            ),
            Self::InvalidConfigPath(path) => write!(
                formatter,
                "diff path {} must remain relative to the repository root",
                path.display()
            ),
        }
    }
}

impl std::error::Error for DiffError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DiffMetricThresholds, DiffThreshold};

    fn analyze_replacement(old: &str, new: &str) -> DiffAnalysis {
        let workspace = ChangedWorkspace {
            root: PathBuf::from("."),
            vcs: DetectedVcs::Git,
            files: vec![ChangedFile {
                kind: ChangeKind::Modified,
                old_path: Some(PathBuf::from("sample.m")),
                new_path: Some(PathBuf::from("sample.m")),
                old_source: Some(old.to_owned()),
                new_source: Some(new.to_owned()),
            }],
        };
        analyze_diff(&workspace, &DiffConfig::default()).expect("analyze")
    }

    #[test]
    fn comment_only_and_syntax_equivalent_formatting_have_zero_executable_churn() {
        let workspace = ChangedWorkspace {
            root: PathBuf::from("."),
            vcs: DetectedVcs::Git,
            files: vec![ChangedFile {
                kind: ChangeKind::Modified,
                old_path: Some(PathBuf::from("sample.m")),
                new_path: Some(PathBuf::from("sample.m")),
                old_source: Some("x=1; % old\n".to_owned()),
                new_source: Some("x = 1; % new\n".to_owned()),
            }],
        };

        let analysis = analyze_diff(&workspace, &DiffConfig::default()).expect("analyze");
        assert_eq!(analysis.metrics.added_executable_lines, 0);
        assert_eq!(analysis.metrics.deleted_executable_lines, 0);
    }

    #[test]
    fn matlab_semantic_whitespace_change_is_not_suppressed() {
        let analysis = analyze_replacement("x = [1 -2];\n", "x = [1-2];\n");
        assert_eq!(analysis.metrics.added_executable_lines, 1);
        assert_eq!(analysis.metrics.deleted_executable_lines, 1);
    }

    #[test]
    fn blank_and_embedded_test_changes_do_not_count_as_executable_churn() {
        let analysis = analyze_replacement(
            "x = 1;\n\n%!test\n%! assert(1 == 1)\n",
            "x = 1;\n\n\n%!test\n%! assert(1 == 2)\n",
        );
        assert_eq!(analysis.metrics.added_executable_lines, 0);
        assert_eq!(analysis.metrics.deleted_executable_lines, 0);
        assert_eq!(analysis.files[0].embedded_test_regions_touched, 1);
    }

    #[test]
    fn conservative_preservation_zones_prevent_whitespace_suppression() {
        for (old, new) in [
            ("disp hello\n", "disp  hello\n"),
            ("q = 1./a;\n", "q = 1. / a;\n"),
            ("x = {1 +1};\n", "x = {1 + 1};\n"),
        ] {
            let analysis = analyze_replacement(old, new);
            assert_eq!(
                analysis.metrics.added_executable_lines, 1,
                "new side should count for {old:?} -> {new:?}"
            );
            assert_eq!(
                analysis.metrics.deleted_executable_lines, 1,
                "old side should count for {old:?} -> {new:?}"
            );
        }
    }

    #[test]
    fn comment_that_starts_with_block_marker_text_does_not_hide_following_code() {
        let analysis = analyze_replacement(
            "%{ explanation, not a block delimiter\nx = 1;\n",
            "%{ explanation, not a block delimiter\nx = 2;\n",
        );
        assert_eq!(analysis.metrics.added_executable_lines, 1);
        assert_eq!(analysis.metrics.deleted_executable_lines, 1);
    }

    #[test]
    fn unrelated_parse_recovery_does_not_disable_formatting_equivalence() {
        let analysis = analyze_replacement("x = 1 + ;\ny=1;\n", "x = 1 + ;\ny = 1;\n");
        assert_eq!(analysis.metrics.added_executable_lines, 0);
        assert_eq!(analysis.metrics.deleted_executable_lines, 0);
        assert_eq!(analysis.files[0].parse_confidence, ParseConfidence::Full);
        assert_eq!(analysis.decision, DiffDecision::Accept);
    }

    #[test]
    fn mixed_formatting_and_semantic_chunk_counts_only_semantic_line() {
        let analysis = analyze_replacement("a=1;\nb=2;\n", "a = 1;\nb=3;\n");
        assert_eq!(analysis.metrics.added_executable_lines, 1);
        assert_eq!(analysis.metrics.deleted_executable_lines, 1);
    }

    #[test]
    fn sparse_large_diff_above_lcs_limit_keeps_exact_churn() {
        let mut old = String::new();
        let mut new = String::new();
        for index in 0..1700 {
            old.push_str(&format!("v{index} = {index};\n"));
            let value = if index == 100 || index == 1600 {
                index + 1
            } else {
                index
            };
            new.push_str(&format!("v{index} = {value};\n"));
        }

        let analysis = analyze_replacement(&old, &new);
        assert_eq!(analysis.metrics.added_executable_lines, 2);
        assert_eq!(analysis.metrics.deleted_executable_lines, 2);
    }

    #[test]
    fn markers_inside_block_comments_are_not_sections_or_embedded_tests() {
        let analysis = analyze_replacement(
            "%{\n%% fake\n%!test\n%}\nx = 1;\n",
            "%{\n%% changed fake\n%! assert(false)\n%}\nx = 1;\n",
        );
        assert_eq!(analysis.metrics.sections_touched, 0);
        assert_eq!(analysis.metrics.sections_added, 0);
        assert_eq!(analysis.files[0].embedded_test_regions_touched, 0);
        assert_eq!(analysis.metrics.added_executable_lines, 0);
        assert_eq!(analysis.metrics.deleted_executable_lines, 0);
    }

    #[test]
    fn fact_cancellation_preserves_operand_order_changes() {
        let analysis = analyze_replacement("x = a - b;\n", "x = b - a;\n");
        assert!(
            analysis.files[0]
                .facts
                .iter()
                .any(|fact| { fact.kind == SemanticFactKind::AssignmentChanged })
        );
        assert!(
            analysis.files[0]
                .facts
                .iter()
                .any(|fact| { fact.kind == SemanticFactKind::NumericExpressionChanged })
        );
    }

    #[test]
    fn thresholds_use_inclusive_review_and_reject_boundaries() {
        let mut decision = DiffDecision::Accept;
        let mut triggers = Vec::new();
        let thresholds = DiffMetricThresholds {
            added_lines: Some(DiffThreshold {
                review: 2,
                reject: 4,
            }),
            ..DiffMetricThresholds::default()
        };

        evaluate_thresholds(
            "general",
            DiffMetrics {
                added_executable_lines: 2,
                ..DiffMetrics::default()
            },
            &thresholds,
            &mut decision,
            &mut triggers,
        );
        assert_eq!(decision, DiffDecision::Review);

        evaluate_thresholds(
            "general",
            DiffMetrics {
                added_executable_lines: 4,
                ..DiffMetrics::default()
            },
            &thresholds,
            &mut decision,
            &mut triggers,
        );
        assert_eq!(decision, DiffDecision::RejectSplit);
    }

    #[test]
    fn every_metric_is_wired_to_inclusive_thresholds() {
        let threshold = Some(DiffThreshold {
            review: 1,
            reject: 2,
        });
        let thresholds = DiffMetricThresholds {
            added_lines: threshold.clone(),
            deleted_lines: threshold.clone(),
            files_touched: threshold.clone(),
            functions_touched: threshold.clone(),
            functions_added: threshold.clone(),
            control_flow: threshold.clone(),
            sections_touched: threshold.clone(),
            sections_added: threshold,
        };
        let review_metrics = DiffMetrics {
            added_executable_lines: 1,
            deleted_executable_lines: 1,
            files_touched: 1,
            functions_touched: 1,
            functions_added: 1,
            control_flow_increase: 1,
            sections_touched: 1,
            sections_added: 1,
        };
        let mut decision = DiffDecision::Accept;
        let mut triggers = Vec::new();
        evaluate_thresholds(
            "general",
            review_metrics,
            &thresholds,
            &mut decision,
            &mut triggers,
        );
        assert_eq!(decision, DiffDecision::Review);
        assert_eq!(triggers.len(), 8);

        let reject_metrics = DiffMetrics {
            added_executable_lines: 2,
            deleted_executable_lines: 2,
            files_touched: 2,
            functions_touched: 2,
            functions_added: 2,
            control_flow_increase: 2,
            sections_touched: 2,
            sections_added: 2,
        };
        let mut decision = DiffDecision::Accept;
        let mut triggers = Vec::new();
        evaluate_thresholds(
            "general",
            reject_metrics,
            &thresholds,
            &mut decision,
            &mut triggers,
        );
        assert_eq!(decision, DiffDecision::RejectSplit);
        assert_eq!(triggers.len(), 8);
    }

    #[test]
    fn each_metric_has_exact_below_review_reject_minus_one_and_reject_boundaries() {
        type MetricSetter = fn(&mut DiffMetrics, usize);
        type ThresholdSetter = fn(&mut DiffMetricThresholds, Option<DiffThreshold>);
        let cases: [(&str, MetricSetter, ThresholdSetter); 8] = [
            (
                "added_lines",
                |metrics, value| metrics.added_executable_lines = value,
                |thresholds, value| thresholds.added_lines = value,
            ),
            (
                "deleted_lines",
                |metrics, value| metrics.deleted_executable_lines = value,
                |thresholds, value| thresholds.deleted_lines = value,
            ),
            (
                "files_touched",
                |metrics, value| metrics.files_touched = value,
                |thresholds, value| thresholds.files_touched = value,
            ),
            (
                "functions_touched",
                |metrics, value| metrics.functions_touched = value,
                |thresholds, value| thresholds.functions_touched = value,
            ),
            (
                "functions_added",
                |metrics, value| metrics.functions_added = value,
                |thresholds, value| thresholds.functions_added = value,
            ),
            (
                "control_flow",
                |metrics, value| metrics.control_flow_increase = value,
                |thresholds, value| thresholds.control_flow = value,
            ),
            (
                "sections_touched",
                |metrics, value| metrics.sections_touched = value,
                |thresholds, value| thresholds.sections_touched = value,
            ),
            (
                "sections_added",
                |metrics, value| metrics.sections_added = value,
                |thresholds, value| thresholds.sections_added = value,
            ),
        ];

        for (metric_name, set_metric, set_threshold) in cases {
            for (value, expected) in [
                (1, DiffDecision::Accept),
                (2, DiffDecision::Review),
                (3, DiffDecision::Review),
                (4, DiffDecision::RejectSplit),
            ] {
                let mut metrics = DiffMetrics::default();
                set_metric(&mut metrics, value);
                let mut thresholds = DiffMetricThresholds::default();
                set_threshold(
                    &mut thresholds,
                    Some(DiffThreshold {
                        review: 2,
                        reject: 4,
                    }),
                );
                let mut decision = DiffDecision::Accept;
                let mut triggers = Vec::new();
                evaluate_thresholds(
                    "general",
                    metrics,
                    &thresholds,
                    &mut decision,
                    &mut triggers,
                );
                assert_eq!(
                    decision, expected,
                    "unexpected boundary result for {metric_name}={value}"
                );
                if expected == DiffDecision::Accept {
                    assert!(triggers.is_empty());
                } else {
                    assert_eq!(triggers.len(), 1);
                    assert_eq!(triggers[0].metric.as_deref(), Some(metric_name));
                }
            }
        }
    }

    #[test]
    fn reject_split_has_stable_json_spelling() {
        assert_eq!(
            serde_json::to_value(DiffDecision::RejectSplit).expect("serialize"),
            serde_json::Value::String("REJECT / SPLIT".to_owned())
        );
    }

    #[test]
    fn context_risk_requires_all_selector_categories_on_one_fact() {
        let risk = DiffRiskConfig {
            id: "DYNAMIC".to_owned(),
            fact: Some(DiffFactKindConfig::CallOrIndexChanged),
            match_mode: DiffRiskMatch::Context,
            targets: vec!["eval".to_owned()],
            functions: vec!["assemble".to_owned()],
            ..DiffRiskConfig::default()
        };
        let fact = SemanticFact {
            kind: SemanticFactKind::CallOrIndexChanged,
            change: ChangeKind::Modified,
            side: DiffSide::New,
            path: "src/a.m".to_owned(),
            range: SourceRange {
                start: SourcePosition { line: 1, column: 1 },
                end: SourcePosition { line: 1, column: 5 },
                start_byte: 0,
                end_byte: 4,
            },
            enclosing_function: Some("assemble".to_owned()),
            identifiers: vec!["eval".to_owned()],
            call_or_index_targets: vec!["eval".to_owned()],
            field_paths: Vec::new(),
            operators: Vec::new(),
            literals: Vec::new(),
        };
        assert!(risk_context_matches(&risk, &fact));

        let mut other = fact.clone();
        other.enclosing_function = Some("other".to_owned());
        assert!(!risk_context_matches(&risk, &other));
    }
}
