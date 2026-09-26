use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::diagnostic::Severity;

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub format: FormatConfig,
    pub lint: LintConfig,
    pub rules: RulesConfig,
    pub exclude: ExcludeConfig,
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
        toml::from_str(source).map_err(ConfigError::Parse)
    }

    pub fn severity(&self, rule_id: &str) -> Severity {
        self.rules
            .severity(rule_id)
            .unwrap_or_else(|| default_severity(rule_id))
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

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    Parse(toml::de::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Parse(error) => write!(formatter, "{error}"),
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
}
