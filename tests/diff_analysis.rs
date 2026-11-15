#![allow(clippy::field_reassign_with_default)]

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use mstyle::config::{
    DiffConfig, DiffFactKindConfig, DiffRiskConfig, DiffRiskMatch, DiffRiskSeverity, DiffThreshold,
    DiffThresholdProfile,
};
use mstyle::diff::{DiffDecision, FileClass, ParseConfidence, SemanticFactKind, analyze_diff};
use mstyle::discovery::{ChangeKind, ChangedFile, ChangedWorkspace, DetectedVcs};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn workspace(files: Vec<ChangedFile>) -> ChangedWorkspace {
    ChangedWorkspace {
        root: PathBuf::from("."),
        vcs: DetectedVcs::Git,
        files,
    }
}

fn modified(path: &str, old: &str, new: &str) -> ChangedFile {
    ChangedFile {
        kind: ChangeKind::Modified,
        old_path: Some(PathBuf::from(path)),
        new_path: Some(PathBuf::from(path)),
        old_source: Some(old.to_owned()),
        new_source: Some(new.to_owned()),
    }
}

fn added(path: &str, source: &str) -> ChangedFile {
    ChangedFile {
        kind: ChangeKind::Added,
        old_path: None,
        new_path: Some(PathBuf::from(path)),
        old_source: None,
        new_source: Some(source.to_owned()),
    }
}

fn deleted(path: &str, source: &str) -> ChangedFile {
    ChangedFile {
        kind: ChangeKind::Deleted,
        old_path: Some(PathBuf::from(path)),
        new_path: None,
        old_source: Some(source.to_owned()),
        new_source: None,
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mstyle-diff-analysis-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&path).expect("create temp directory");
    path
}

#[test]
fn reports_structural_spread_sections_and_test_metrics_separately() {
    let files = vec![
        modified(
            "src/core.m",
            "function y = core(x)\n  y = x;\nend\n",
            concat!(
                "function y = core(x)\n",
                "  if x > 0\n",
                "    y = x;\n",
                "  else\n",
                "    y = 0;\n",
                "  end\n",
                "end\n",
            ),
        ),
        added("tests/core_test.m", "assert(core(1) == 1);\n"),
        added("generated/codegen.m", "generated = 1;\n"),
        modified(
            "src/sections.m",
            "%% First\nx = 1;\n",
            "%% First\nx = 2;\n%% Second\ny = 1;\n",
        ),
    ];
    let mut config = DiffConfig::default();
    config.test_paths = vec!["tests/**".to_owned()];
    config.generated_paths = vec!["generated/**".to_owned()];
    config.thresholds.general.files_touched = Some(DiffThreshold {
        review: 2,
        reject: 5,
    });
    config.thresholds.profile.push(DiffThresholdProfile {
        name: "source".to_owned(),
        paths: vec!["src/**".to_owned()],
        functions_touched: Some(DiffThreshold {
            review: 1,
            reject: 3,
        }),
        ..DiffThresholdProfile::default()
    });

    let analysis = analyze_diff(&workspace(files), &config).expect("analyze");

    assert_eq!(analysis.metrics.files_touched, 2);
    assert_eq!(analysis.test_metrics.files_touched, 1);
    assert_eq!(analysis.generated_files, vec!["generated/codegen.m"]);
    assert_eq!(analysis.decision, DiffDecision::Review);
    assert!(analysis.metrics.functions_touched >= 1);
    assert!(analysis.metrics.control_flow_increase >= 1);
    assert!(analysis.metrics.sections_touched >= 1);
    assert!(analysis.metrics.sections_added >= 1);
    assert_eq!(analysis.profiles.len(), 1);
    assert_eq!(analysis.profiles[0].name, "source");
    assert!(analysis.triggers.iter().any(|trigger| {
        trigger
            .id
            .contains("THRESHOLD.general.files_touched.REVIEW")
    }));
    assert!(analysis.triggers.iter().any(|trigger| {
        trigger
            .id
            .contains("THRESHOLD.profile:source.functions_touched.REVIEW")
    }));
}

#[test]
fn changed_recovery_forces_review_but_unrelated_recovery_does_not() {
    let unrelated = analyze_diff(
        &workspace(vec![modified(
            "broken.m",
            "x = 1 + ;\ny = 1;\n",
            "x = 1 + ;\ny = 2;\n",
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze unrelated recovery");

    assert_eq!(unrelated.files[0].parse_confidence, ParseConfidence::Full);
    assert_eq!(unrelated.decision, DiffDecision::Accept);

    let overlapping = analyze_diff(
        &workspace(vec![modified(
            "broken.m",
            "x = 1;\ny = 1;\n",
            "x = 1;\ny = 2 + ;\n",
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze overlapping recovery");

    assert_eq!(
        overlapping.files[0].parse_confidence,
        ParseConfidence::Recovered
    );
    assert_eq!(overlapping.decision, DiffDecision::Review);
    assert!(overlapping.source_diagnostics);
    assert_eq!(overlapping.metrics.functions_touched, 0);
    assert_eq!(overlapping.metrics.functions_added, 0);
    assert_eq!(overlapping.metrics.control_flow_increase, 0);
}

#[test]
fn recovered_region_does_not_hide_clean_structural_metrics_elsewhere() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "mixed_recovery.m",
            concat!(
                "function y = broken(x)\n",
                "  y = x;\n",
                "end\n",
                "function z = clean(x)\n",
                "  z = x;\n",
                "end\n",
            ),
            concat!(
                "function y = broken(x)\n",
                "  y = x + ;\n",
                "end\n",
                "function z = clean(x)\n",
                "  if x > 0\n",
                "    z = x;\n",
                "  else\n",
                "    z = 0;\n",
                "  end\n",
                "end\n",
            ),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(
        analysis.files[0].parse_confidence,
        ParseConfidence::Recovered
    );
    assert_eq!(analysis.decision, DiffDecision::Review);
    assert_eq!(analysis.metrics.functions_touched, 1);
    assert_eq!(analysis.metrics.control_flow_increase, 1);
    assert!(analysis.files[0].functions.iter().any(|function| {
        function.name == "clean" && function.change == mstyle::diff::FunctionChangeKind::Touched
    }));
}

#[test]
fn zero_width_missing_at_changed_boundary_forces_review() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "missing_end.m",
            "x = 1;\n",
            "x = 1;\nif x > 0\n",
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(
        analysis.files[0].parse_confidence,
        ParseConfidence::Recovered
    );
    assert_eq!(analysis.decision, DiffDecision::Review);
}

#[test]
fn ambiguous_parentheses_emit_call_or_index_fact_with_target() {
    let analysis = analyze_diff(
        &workspace(vec![modified("ambiguous.m", "x = A(1);\n", "x = A(2);\n")]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    let fact = analysis.files[0]
        .facts
        .iter()
        .find(|fact| fact.kind == SemanticFactKind::CallOrIndexChanged)
        .expect("call/index fact");
    assert!(
        fact.call_or_index_targets
            .iter()
            .any(|target| target == "A")
    );
}

#[test]
fn contextual_risk_matches_one_fact_occurrence_and_raises_policy() {
    let mut config = DiffConfig::default();
    config.risk.push(DiffRiskConfig {
        id: "DYNAMIC_EVALUATION_CHANGE".to_owned(),
        severity: DiffRiskSeverity::Review,
        fact: Some(DiffFactKindConfig::CallOrIndexChanged),
        match_mode: DiffRiskMatch::Context,
        targets: vec!["eval".to_owned()],
        functions: vec!["assemble".to_owned()],
        ..DiffRiskConfig::default()
    });
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "assemble.m",
            concat!("function y = assemble(x)\n", "  y = eval(x);\n", "end\n",),
            concat!(
                "function y = assemble(x)\n",
                "  y = eval(x + 1);\n",
                "end\n",
            ),
        )]),
        &config,
    )
    .expect("analyze");

    assert_eq!(analysis.decision, DiffDecision::Review);
    assert!(
        analysis
            .triggers
            .iter()
            .any(|trigger| { trigger.id == "DYNAMIC_EVALUATION_CHANGE" })
    );
}

#[test]
fn contextual_selectors_do_not_match_across_distinct_fact_occurrences() {
    let mut config = DiffConfig::default();
    config.risk.push(DiffRiskConfig {
        id: "SPLIT_CONTEXT".to_owned(),
        severity: DiffRiskSeverity::Reject,
        facts: vec![
            DiffFactKindConfig::CallOrIndexChanged,
            DiffFactKindConfig::AssignmentChanged,
        ],
        match_mode: DiffRiskMatch::Context,
        identifiers: vec!["state".to_owned()],
        targets: vec!["eval".to_owned()],
        ..DiffRiskConfig::default()
    });
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "split_context.m",
            "value = eval(x);\nstate = 1;\n",
            "value = eval(y);\nstate = 2;\n",
        )]),
        &config,
    )
    .expect("analyze");

    assert!(
        !analysis
            .triggers
            .iter()
            .any(|trigger| trigger.id == "SPLIT_CONTEXT")
    );
    assert_eq!(analysis.decision, DiffDecision::Accept);
}

#[test]
fn public_api_inventory_is_rooted_and_normalized() {
    let root = temp_dir("public-api");
    fs::write(root.join("api-files.txt"), "foo.m\n").expect("inventory");

    let mut config = DiffConfig::default();
    config.public_api_inventory = Some(PathBuf::from("api-files.txt"));
    config.public_api_root = PathBuf::from("src");
    let workspace = ChangedWorkspace {
        root: root.clone(),
        vcs: DetectedVcs::Git,
        files: vec![modified(
            "src/foo.m",
            "function y = foo(x)\n  y = x;\nend\n",
            "function y = foo(x, scale)\n  y = x;\nend\n",
        )],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert!(
        analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::PublicApiSignatureChanged })
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn generated_and_test_classification_is_stable_in_json() {
    let mut config = DiffConfig::default();
    config.test_paths = vec!["tests/**".to_owned()];
    let analysis = analyze_diff(
        &workspace(vec![
            added("src/a.m", "x = 1;\n"),
            added("tests/a_test.m", "assert(true);\n"),
        ]),
        &config,
    )
    .expect("analyze");

    assert_eq!(analysis.files[0].classification, FileClass::Implementation);
    assert_eq!(analysis.files[1].classification, FileClass::Test);
    let json = serde_json::to_value(&analysis).expect("serialize analysis");
    assert_eq!(json["decision"], "ACCEPT");
    assert!(
        json["files"][0]["facts"]
            .as_array()
            .expect("facts")
            .iter()
            .all(|fact| fact["kind"].as_str().is_some())
    );
}

#[test]
fn pure_deletion_counts_old_executable_lines_and_function_deletion() {
    let analysis = analyze_diff(
        &workspace(vec![deleted(
            "removed.m",
            "function y = removed(x)\n  y = x + 1;\nend\n",
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(analysis.metrics.added_executable_lines, 0);
    assert!(analysis.metrics.deleted_executable_lines >= 1);
    assert!(
        analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::FunctionDeleted })
    );
}

#[test]
fn local_function_change_maps_to_the_local_function() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "functions.m",
            concat!(
                "function y = primary(x)\n",
                "  y = local_helper(x);\n",
                "end\n",
                "function y = local_helper(x)\n",
                "  y = x + 1;\n",
                "end\n",
            ),
            concat!(
                "function y = primary(x)\n",
                "  y = local_helper(x);\n",
                "end\n",
                "function y = local_helper(x)\n",
                "  y = x + 2;\n",
                "end\n",
            ),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert!(
        analysis.files[0]
            .functions
            .iter()
            .any(|function| { function.name == "local_helper" })
    );
    assert!(!analysis.files[0].functions.iter().any(|function| {
        function.change == mstyle::diff::FunctionChangeKind::Touched && function.name == "primary"
    }));
}

#[test]
fn rename_matches_old_and_new_path_profiles_independently() {
    let workspace = ChangedWorkspace {
        root: PathBuf::from("."),
        vcs: DetectedVcs::Git,
        files: vec![ChangedFile {
            kind: ChangeKind::Renamed,
            old_path: Some(PathBuf::from("old/a.m")),
            new_path: Some(PathBuf::from("new/a.m")),
            old_source: Some("x = 1;\n".to_owned()),
            new_source: Some("x = 2;\n".to_owned()),
        }],
    };
    let mut config = DiffConfig::default();
    for (name, path) in [("old", "old/**"), ("new", "new/**")] {
        config.thresholds.profile.push(DiffThresholdProfile {
            name: name.to_owned(),
            paths: vec![path.to_owned()],
            files_touched: Some(DiffThreshold {
                review: 1,
                reject: 2,
            }),
            ..DiffThresholdProfile::default()
        });
    }

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert_eq!(analysis.profiles.len(), 2);
    assert!(
        analysis
            .profiles
            .iter()
            .any(|profile| profile.name == "old")
    );
    assert!(
        analysis
            .profiles
            .iter()
            .any(|profile| profile.name == "new")
    );
    assert_eq!(analysis.decision, DiffDecision::Review);
}

#[test]
fn local_signature_change_does_not_emit_public_api_signature_fact() {
    let root = temp_dir("public-api-local");
    fs::write(root.join("api-files.txt"), "foo.m\n").expect("inventory");
    let mut config = DiffConfig::default();
    config.public_api_inventory = Some(PathBuf::from("api-files.txt"));
    config.public_api_root = PathBuf::from("src");
    let workspace = ChangedWorkspace {
        root: root.clone(),
        vcs: DetectedVcs::Git,
        files: vec![modified(
            "src/foo.m",
            concat!(
                "function y = foo(x)\n",
                "  y = helper(x);\n",
                "end\n",
                "function y = helper(x)\n",
                "  y = x;\n",
                "end\n",
            ),
            concat!(
                "function y = foo(x)\n",
                "  y = helper(x, 1);\n",
                "end\n",
                "function y = helper(x, scale)\n",
                "  y = x;\n",
                "end\n",
            ),
        )],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert!(
        !analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::PublicApiSignatureChanged })
    );
    assert!(analysis.files[0].facts.iter().any(|fact| {
        fact.kind == SemanticFactKind::FunctionSignatureChanged
            && fact.enclosing_function.as_deref() == Some("helper")
    }));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn generated_and_test_classification_follow_the_current_rename_path() {
    let mut config = DiffConfig::default();
    config.generated_paths = vec!["generated/**".to_owned()];
    config.test_paths = vec!["tests/**".to_owned()];
    let workspace = ChangedWorkspace {
        root: PathBuf::from("."),
        vcs: DetectedVcs::Git,
        files: vec![
            ChangedFile {
                kind: ChangeKind::Renamed,
                old_path: Some(PathBuf::from("generated/a.m")),
                new_path: Some(PathBuf::from("src/a.m")),
                old_source: Some("x = 1;\n".to_owned()),
                new_source: Some("x = 2;\n".to_owned()),
            },
            ChangedFile {
                kind: ChangeKind::Renamed,
                old_path: Some(PathBuf::from("tests/b.m")),
                new_path: Some(PathBuf::from("src/b.m")),
                old_source: Some("y = 1;\n".to_owned()),
                new_source: Some("y = 2;\n".to_owned()),
            },
            ChangedFile {
                kind: ChangeKind::Renamed,
                old_path: Some(PathBuf::from("src/c.m")),
                new_path: Some(PathBuf::from("generated/c.m")),
                old_source: Some("z = 1;\n".to_owned()),
                new_source: Some("z = 2;\n".to_owned()),
            },
        ],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert_eq!(analysis.metrics.files_touched, 2);
    assert_eq!(analysis.test_metrics.files_touched, 0);
    assert_eq!(analysis.generated_files, vec!["generated/c.m"]);
    assert!(
        analysis
            .files
            .iter()
            .all(|file| { file.classification == FileClass::Implementation })
    );
}

#[test]
fn unchanged_sibling_call_does_not_emit_call_or_index_fact() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "siblings.m",
            "x = 1; y = eval(z);\n",
            "x = 2; y = eval(z);\n",
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert!(!analysis.files[0].facts.iter().any(|fact| {
        fact.kind == SemanticFactKind::CallOrIndexChanged
            && fact
                .call_or_index_targets
                .iter()
                .any(|target| target == "eval")
    }));
}

#[test]
fn body_only_edit_does_not_change_control_flow_complexity_or_fact() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "branch.m",
            concat!("if x > 0\n", "  y = 1;\n", "end\n",),
            concat!("if x > 0\n", "  y = 2;\n", "end\n",),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(analysis.metrics.control_flow_increase, 0);
    assert!(
        !analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::ControlFlowChanged })
    );
}

#[test]
fn adding_one_if_adds_one_control_flow_point() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "branch.m",
            "y = x;\n",
            concat!("if x > 0\n", "  y = x;\n", "end\n",),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(analysis.metrics.control_flow_increase, 1);
}

#[test]
fn public_api_signature_change_survives_rename_out_of_inventory_path() {
    let root = temp_dir("public-api-rename");
    fs::write(root.join("api-files.txt"), "foo.m\n").expect("inventory");
    let mut config = DiffConfig::default();
    config.public_api_inventory = Some(PathBuf::from("api-files.txt"));
    config.public_api_root = PathBuf::from("src");
    let workspace = ChangedWorkspace {
        root: root.clone(),
        vcs: DetectedVcs::Git,
        files: vec![ChangedFile {
            kind: ChangeKind::Renamed,
            old_path: Some(PathBuf::from("src/foo.m")),
            new_path: Some(PathBuf::from("moved/foo.m")),
            old_source: Some("function y = foo(x)\n  y = x;\nend\n".to_owned()),
            new_source: Some("function y = foo(x, scale)\n  y = x * scale;\nend\n".to_owned()),
        }],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert!(
        analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::PublicApiSignatureChanged })
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn multiline_primary_function_signature_change_is_detected() {
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "multiline.m",
            concat!(
                "function y = multiline( ...\n",
                "    x, ...\n",
                "    scale)\n",
                "  y = x * scale;\n",
                "end\n",
            ),
            concat!(
                "function y = multiline( ...\n",
                "    x, ...\n",
                "    scale, offset)\n",
                "  y = x * scale + offset;\n",
                "end\n",
            ),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert!(analysis.files[0].facts.iter().any(|fact| {
        fact.kind == SemanticFactKind::FunctionSignatureChanged
            && fact.enclosing_function.as_deref() == Some("multiline")
    }));
}

#[test]
fn inventoried_script_local_function_is_not_public_api_primary() {
    let root = temp_dir("public-api-script");
    fs::write(root.join("api-files.txt"), "script.m\n").expect("inventory");
    let mut config = DiffConfig::default();
    config.public_api_inventory = Some(PathBuf::from("api-files.txt"));
    config.public_api_root = PathBuf::from("src");
    let workspace = ChangedWorkspace {
        root: root.clone(),
        vcs: DetectedVcs::Git,
        files: vec![modified(
            "src/script.m",
            concat!(
                "x = 1;\n",
                "function y = helper(a)\n",
                "  y = a;\n",
                "end\n",
            ),
            concat!(
                "x = 1;\n",
                "function y = helper(a, scale)\n",
                "  y = a * scale;\n",
                "end\n",
            ),
        )],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert!(
        !analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::PublicApiSignatureChanged })
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn inventoried_class_method_signature_is_not_public_api_primary() {
    let root = temp_dir("public-api-class");
    fs::write(root.join("api-files.txt"), "Widget.m\n").expect("inventory");
    let mut config = DiffConfig::default();
    config.public_api_inventory = Some(PathBuf::from("api-files.txt"));
    config.public_api_root = PathBuf::from("src");
    let workspace = ChangedWorkspace {
        root: root.clone(),
        vcs: DetectedVcs::Git,
        files: vec![modified(
            "src/Widget.m",
            concat!(
                "classdef Widget\n",
                "  methods\n",
                "    function y = value(obj, x)\n",
                "      y = x;\n",
                "    end\n",
                "  end\n",
                "end\n",
            ),
            concat!(
                "classdef Widget\n",
                "  methods\n",
                "    function y = value(obj, x, scale)\n",
                "      y = x * scale;\n",
                "    end\n",
                "  end\n",
                "end\n",
            ),
        )],
    };

    let analysis = analyze_diff(&workspace, &config).expect("analyze");
    assert!(
        !analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::PublicApiSignatureChanged })
    );
    assert!(
        analysis.files[0]
            .facts
            .iter()
            .any(|fact| { fact.kind == SemanticFactKind::FunctionSignatureChanged })
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn standalone_abstract_method_signature_is_counted_as_a_function() {
    let analysis = analyze_diff(
        &workspace(vec![added(
            "AbstractWidget.m",
            concat!(
                "classdef (Abstract) AbstractWidget\n",
                "  methods (Abstract)\n",
                "    y = value(obj, x)\n",
                "  end\n",
                "end\n",
            ),
        )]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert!(analysis.metrics.functions_added >= 1);
    assert!(analysis.files[0].functions.iter().any(|function| {
        function.name == "value" && function.change == mstyle::diff::FunctionChangeKind::Added
    }));
}

#[test]
fn equal_line_churn_exposes_structural_spread() {
    let compact = analyze_diff(
        &workspace(vec![modified(
            "one.m",
            concat!(
                "function y = one(x)\n",
                "  a = x + 1;\n",
                "  y = a + 1;\n",
                "end\n",
            ),
            concat!(
                "function y = one(x)\n",
                "  a = x + 2;\n",
                "  y = a + 2;\n",
                "end\n",
            ),
        )]),
        &DiffConfig::default(),
    )
    .expect("compact analysis");
    let spread = analyze_diff(
        &workspace(vec![
            modified(
                "a.m",
                "function y = a(x)\n  y = x + 1;\nend\n",
                "function y = a(x)\n  y = x + 2;\nend\n",
            ),
            modified(
                "b.m",
                "function y = b(x)\n  y = x + 1;\nend\n",
                "function y = b(x)\n  y = x + 2;\nend\n",
            ),
        ]),
        &DiffConfig::default(),
    )
    .expect("spread analysis");

    assert_eq!(
        compact.metrics.added_executable_lines,
        spread.metrics.added_executable_lines
    );
    assert_eq!(
        compact.metrics.deleted_executable_lines,
        spread.metrics.deleted_executable_lines
    );
    assert_eq!(compact.metrics.files_touched, 1);
    assert_eq!(spread.metrics.files_touched, 2);
    assert_eq!(compact.metrics.functions_touched, 1);
    assert_eq!(spread.metrics.functions_touched, 2);
}

#[test]
fn overlapping_profiles_all_apply_and_highest_severity_wins() {
    let mut config = DiffConfig::default();
    config.thresholds.profile.push(DiffThresholdProfile {
        name: "broad".to_owned(),
        paths: vec!["src/**".to_owned()],
        added_lines: Some(DiffThreshold {
            review: 1,
            reject: 3,
        }),
        ..DiffThresholdProfile::default()
    });
    config.thresholds.profile.push(DiffThresholdProfile {
        name: "sensitive".to_owned(),
        paths: vec!["src/numerics/**".to_owned()],
        added_lines: Some(DiffThreshold {
            review: 0,
            reject: 1,
        }),
        ..DiffThresholdProfile::default()
    });
    let analysis = analyze_diff(
        &workspace(vec![modified(
            "src/numerics/kernel.m",
            "x = 1;\n",
            "x = 2;\n",
        )]),
        &config,
    )
    .expect("analyze");

    assert_eq!(analysis.profiles.len(), 2);
    assert_eq!(analysis.decision, DiffDecision::RejectSplit);
    assert!(analysis.triggers.iter().any(|trigger| {
        trigger.scope == "profile:broad" && trigger.severity == DiffDecision::Review
    }));
    assert!(analysis.triggers.iter().any(|trigger| {
        trigger.scope == "profile:sensitive" && trigger.severity == DiffDecision::RejectSplit
    }));
}

#[test]
fn analyzer_defensively_ignores_non_matlab_records() {
    let analysis = analyze_diff(
        &workspace(vec![
            modified("source.m", "x = 1;\n", "x = 2;\n"),
            ChangedFile {
                kind: ChangeKind::Modified,
                old_path: Some(PathBuf::from("notes.txt")),
                new_path: Some(PathBuf::from("notes.txt")),
                old_source: Some("old\n".to_owned()),
                new_source: Some("new\n".to_owned()),
            },
        ]),
        &DiffConfig::default(),
    )
    .expect("analyze");

    assert_eq!(analysis.metrics.files_touched, 1);
    assert_eq!(analysis.files.len(), 1);
    assert_eq!(analysis.files[0].new_path.as_deref(), Some("source.m"));
}
