use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::collections::BTreeSet;

use serde::Deserialize;

use crate::diagnostic::Severity;

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub format: FormatConfig,
    pub lint: LintConfig,
    pub rules: RulesConfig,
    pub exclude: ExcludeConfig,
    pub diff: DiffConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let source = fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::parse(&source)
    }

    pub fn load_if_exists(path: &Path) -> Result<Self, ConfigError> {
        match fs::read_to_string(path) {
            Ok(source) => Self::parse(&source),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(ConfigError::Io(error)),
        }
    }

    fn parse(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn severity(&self, rule_id: &str) -> Severity {
        self.rules
            .severity(rule_id)
            .unwrap_or_else(|| default_severity(rule_id))
    }

    fn validate(&self) -> Result<(), ConfigError> {
        self.diff.validate()
    }
}

fn default_severity(rule_id: &str) -> Severity {
    if rule_id.starts_with('F') || rule_id == "PARSE" {
        Severity::Error
    } else {
        Severity::Warning
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FormatConfig {
    pub indent_width: usize,
    pub line_endings: LineEndings,
}

impl Default for FormatConfig {
    fn default() -> Self {
        Self {
            indent_width: 2,
            line_endings: LineEndings::Lf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineEndings {
    Lf,
    Crlf,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LintConfig {
    pub line_length: usize,
}

impl Default for LintConfig {
    fn default() -> Self {
        Self { line_length: 120 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RulesConfig {
    #[serde(rename = "F001")]
    pub f001: Option<SeveritySetting>,
    #[serde(rename = "F002")]
    pub f002: Option<SeveritySetting>,
    #[serde(rename = "F003")]
    pub f003: Option<SeveritySetting>,
    #[serde(rename = "F004")]
    pub f004: Option<SeveritySetting>,
    #[serde(rename = "F005")]
    pub f005: Option<SeveritySetting>,
    #[serde(rename = "F006")]
    pub f006: Option<SeveritySetting>,
    #[serde(rename = "F007")]
    pub f007: Option<SeveritySetting>,
    #[serde(rename = "F008")]
    pub f008: Option<SeveritySetting>,
    #[serde(rename = "F009")]
    pub f009: Option<SeveritySetting>,
    #[serde(rename = "F010")]
    pub f010: Option<SeveritySetting>,
    #[serde(rename = "F011")]
    pub f011: Option<SeveritySetting>,
    #[serde(rename = "L001")]
    pub l001: Option<SeveritySetting>,
    #[serde(rename = "L002")]
    pub l002: Option<SeveritySetting>,
}

impl RulesConfig {
    fn severity(&self, rule_id: &str) -> Option<Severity> {
        let setting = match rule_id {
            "F001" => self.f001,
            "F002" => self.f002,
            "F003" => self.f003,
            "F004" => self.f004,
            "F005" => self.f005,
            "F006" => self.f006,
            "F007" => self.f007,
            "F008" => self.f008,
            "F009" => self.f009,
            "F010" => self.f010,
            "F011" => self.f011,
            "L001" => self.l001,
            "L002" => self.l002,
            _ => None,
        }?;

        Some(setting.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeveritySetting {
    Error,
    Warning,
}

impl From<SeveritySetting> for Severity {
    fn from(value: SeveritySetting) -> Self {
        match value {
            SeveritySetting::Error => Self::Error,
            SeveritySetting::Warning => Self::Warning,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExcludeConfig {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiffConfig {
    pub vcs: DiffVcs,
    pub advisory: bool,
    pub generated_paths: Vec<String>,
    pub test_paths: Vec<String>,
    pub public_api_inventory: Option<PathBuf>,
    pub public_api_root: PathBuf,
    pub thresholds: DiffThresholdsConfig,
    pub risk: Vec<DiffRiskConfig>,
}

impl Default for DiffConfig {
    fn default() -> Self {
        Self {
            vcs: DiffVcs::Auto,
            advisory: true,
            generated_paths: Vec::new(),
            test_paths: Vec::new(),
            public_api_inventory: None,
            public_api_root: PathBuf::from("."),
            thresholds: DiffThresholdsConfig::default(),
            risk: Vec::new(),
        }
    }
}

impl DiffConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.thresholds.general.validate("diff.thresholds.general")?;
        let mut profile_names = BTreeSet::new();
        for profile in &self.thresholds.profile {
            if profile.name.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "diff threshold profile name must not be empty".to_owned(),
                ));
            }
            if !profile_names.insert(profile.name.clone()) {
                return Err(ConfigError::Validation(format!(
                    "duplicate diff threshold profile name {:?}",
                    profile.name
                )));
            }
            if profile.paths.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "diff threshold profile {:?} must configure at least one path",
                    profile.name
                )));
            }
            profile
                .metrics()
                .validate(&format!("diff.thresholds.profile {:?}", profile.name))?;
        }

        for risk in &self.risk {
            if risk.id.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "diff risk id must not be empty".to_owned(),
                ));
            }
            if risk.fact.is_none() && risk.facts.is_empty() {
                return Err(ConfigError::Validation(format!(
                    "diff risk {:?} must configure fact or facts",
                    risk.id
                )));
            }
            if risk.match_mode == DiffRiskMatch::Context
                && risk.identifiers.is_empty()
                && risk.targets.is_empty()
                && risk.field_paths.is_empty()
                && risk.functions.is_empty()
                && risk.operators.is_empty()
                && risk.literals.is_empty()
            {
                return Err(ConfigError::Validation(format!(
                    "diff risk {:?} uses match = \"context\" without a context selector",
                    risk.id
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffVcs {
    Auto,
    Git,
    Fossil,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiffThresholdsConfig {
    pub general: DiffMetricThresholds,
    pub profile: Vec<DiffThresholdProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiffThresholdProfile {
    pub name: String,
    pub paths: Vec<String>,
    pub added_lines: Option<DiffThreshold>,
    pub deleted_lines: Option<DiffThreshold>,
    pub files_touched: Option<DiffThreshold>,
    pub functions_touched: Option<DiffThreshold>,
    pub functions_added: Option<DiffThreshold>,
    pub control_flow: Option<DiffThreshold>,
    pub sections_touched: Option<DiffThreshold>,
    pub sections_added: Option<DiffThreshold>,
}

impl DiffThresholdProfile {
    pub fn metrics(&self) -> DiffMetricThresholds {
        DiffMetricThresholds {
            added_lines: self.added_lines.clone(),
            deleted_lines: self.deleted_lines.clone(),
            files_touched: self.files_touched.clone(),
            functions_touched: self.functions_touched.clone(),
            functions_added: self.functions_added.clone(),
            control_flow: self.control_flow.clone(),
            sections_touched: self.sections_touched.clone(),
            sections_added: self.sections_added.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DiffMetricThresholds {
    pub added_lines: Option<DiffThreshold>,
    pub deleted_lines: Option<DiffThreshold>,
    pub files_touched: Option<DiffThreshold>,
    pub functions_touched: Option<DiffThreshold>,
    pub functions_added: Option<DiffThreshold>,
    pub control_flow: Option<DiffThreshold>,
    pub sections_touched: Option<DiffThreshold>,
    pub sections_added: Option<DiffThreshold>,
}

impl DiffMetricThresholds {
    fn validate(&self, context: &str) -> Result<(), ConfigError> {
        for (name, threshold) in [
            ("added_lines", self.added_lines.as_ref()),
            ("deleted_lines", self.deleted_lines.as_ref()),
            ("files_touched", self.files_touched.as_ref()),
            ("functions_touched", self.functions_touched.as_ref()),
            ("functions_added", self.functions_added.as_ref()),
            ("control_flow", self.control_flow.as_ref()),
            ("sections_touched", self.sections_touched.as_ref()),
            ("sections_added", self.sections_added.as_ref()),
        ] {
            if let Some(threshold) = threshold {
                threshold.validate(context, name)?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffThreshold {
    pub review: usize,
    pub reject: usize,
}

impl DiffThreshold {
    fn validate(&self, context: &str, metric: &str) -> Result<(), ConfigError> {
        if self.review >= self.reject {
            return Err(ConfigError::Validation(format!(
                "{context}.{metric} requires review < reject"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffFactKindConfig {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiffRiskSeverity {
    Review,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DiffRiskMatch {
    #[default]
    Any,
    Context,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiffRiskConfig {
    pub id: String,
    pub severity: DiffRiskSeverity,
    #[serde(default)]
    pub fact: Option<DiffFactKindConfig>,
    #[serde(default)]
    pub facts: Vec<DiffFactKindConfig>,
    #[serde(default, rename = "match")]
    pub match_mode: DiffRiskMatch,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub identifiers: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub field_paths: Vec<String>,
    #[serde(default)]
    pub functions: Vec<String>,
    #[serde(default)]
    pub operators: Vec<String>,
    #[serde(default)]
    pub literals: Vec<String>,
}

impl Default for DiffRiskConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            severity: DiffRiskSeverity::Review,
            fact: None,
            facts: Vec::new(),
            match_mode: DiffRiskMatch::Any,
            paths: Vec::new(),
            identifiers: Vec::new(),
            targets: Vec::new(),
            field_paths: Vec::new(),
            functions: Vec::new(),
            operators: Vec::new(),
            literals: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(toml::de::Error),
    Validation(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Parse(error) => write!(formatter, "{error}"),
            Self::Validation(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_uses_defaults() {
        let config = Config::parse("").expect("empty config should be valid");

        assert_eq!(config.format.indent_width, 2);
        assert_eq!(config.format.line_endings, LineEndings::Lf);
        assert_eq!(config.lint.line_length, 120);
        assert_eq!(config.severity("F001"), Severity::Error);
        assert_eq!(config.severity("L001"), Severity::Warning);
        assert!(config.exclude.paths.is_empty());
        assert_eq!(config.diff.vcs, DiffVcs::Auto);
        assert!(config.diff.advisory);
        assert!(config.diff.thresholds.profile.is_empty());
        assert!(config.diff.risk.is_empty());
    }

    #[test]
    fn parses_supported_settings_and_rule_severity() {
        let config = Config::parse(
            r#"
                [format]
                indent_width = 4
                line_endings = "crlf"

                [lint]
                line_length = 100

                [rules]
                L001 = "error"
                F007 = "warning"

                [exclude]
                paths = ["vendor/**", "build/**"]
            "#,
        )
        .expect("supported config should parse");

        assert_eq!(config.format.indent_width, 4);
        assert_eq!(config.format.line_endings, LineEndings::Crlf);
        assert_eq!(config.lint.line_length, 100);
        assert_eq!(config.severity("L001"), Severity::Error);
        assert_eq!(config.severity("F007"), Severity::Warning);
        assert_eq!(config.exclude.paths, ["vendor/**", "build/**"]);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let error = Config::parse("unknown = true").expect_err("unknown keys must fail");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_unknown_rule_ids() {
        let error = Config::parse(
            r#"
                [rules]
                L999 = "warning"
            "#,
        )
        .expect_err("unknown rule IDs must fail");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn parses_diff_policy_configuration() {
        let config = Config::parse(
            r#"
                [diff]
                vcs = "git"
                advisory = false
                generated_paths = ["generated/**"]
                test_paths = ["tests/**"]
                public_api_inventory = "api-files.txt"
                public_api_root = "src"

                [diff.thresholds.general]
                added_lines = { review = 10, reject = 20 }

                [[diff.thresholds.profile]]
                name = "sensitive"
                paths = ["src/numerics/**"]
                functions_touched = { review = 2, reject = 4 }

                [[diff.risk]]
                id = "DYNAMIC_EVALUATION_CHANGE"
                severity = "review"
                facts = ["call_or_index_changed"]
                targets = ["eval", "evalin"]
                match = "context"
            "#,
        )
        .expect("diff config should parse");

        assert_eq!(config.diff.vcs, DiffVcs::Git);
        assert!(!config.diff.advisory);
        assert_eq!(
            config.diff.thresholds.general.added_lines,
            Some(DiffThreshold {
                review: 10,
                reject: 20
            })
        );
        assert_eq!(config.diff.thresholds.profile[0].name, "sensitive");
        assert_eq!(config.diff.risk[0].id, "DYNAMIC_EVALUATION_CHANGE");
        assert_eq!(config.diff.risk[0].match_mode, DiffRiskMatch::Context);
    }

    #[test]
    fn rejects_invalid_diff_threshold_boundaries() {
        let error = Config::parse(
            r#"
                [diff.thresholds.general]
                added_lines = { review = 10, reject = 10 }
            "#,
        )
        .expect_err("review must be lower than reject");

        assert!(error.to_string().contains("review < reject"));
    }

    #[test]
    fn rejects_unknown_diff_profile_keys() {
        let error = Config::parse(
            r#"
                [[diff.thresholds.profile]]
                name = "sensitive"
                paths = ["src/**"]
                typo_metric = { review = 1, reject = 2 }
            "#,
        )
        .expect_err("unknown profile keys must fail");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_context_risk_without_context_selectors() {
        let error = Config::parse(
            r#"
                [[diff.risk]]
                id = "INVALID"
                severity = "review"
                fact = "assignment_changed"
                match = "context"
            "#,
        )
        .expect_err("context match needs a selector");

        assert!(error.to_string().contains("without a context selector"));
    }

    #[test]
    fn requires_diff_risk_severity() {
        let error = Config::parse(
            r#"
                [[diff.risk]]
                id = "MISSING_SEVERITY"
                fact = "assignment_changed"
            "#,
        )
        .expect_err("risk severity should be required");

        assert!(error.to_string().contains("missing field"));
    }

    #[test]
    fn rejects_duplicate_diff_profile_names() {
        let error = Config::parse(
            r#"
                [[diff.thresholds.profile]]
                name = "sensitive"
                paths = ["src/a/**"]
                added_lines = { review = 1, reject = 2 }

                [[diff.thresholds.profile]]
                name = "sensitive"
                paths = ["src/b/**"]
                added_lines = { review = 1, reject = 2 }
            "#,
        )
        .expect_err("profile names must be unique");

        assert!(error.to_string().contains("duplicate diff threshold profile"));
    }
}
