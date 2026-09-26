use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub format: FormatConfig,
    pub lint: LintConfig,
    pub rules: RulesConfig,
    pub exclude: ExcludeConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            format: FormatConfig::default(),
            lint: LintConfig::default(),
            rules: RulesConfig {},
            exclude: ExcludeConfig::default(),
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct RulesConfig {}

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
        assert!(config.exclude.paths.is_empty());
    }

    #[test]
    fn parses_supported_settings() {
        let config = Config::parse(
            r#"
                [format]
                indent_width = 4
                line_endings = "crlf"

                [lint]
                line_length = 100

                [exclude]
                paths = ["vendor/**", "build/**"]
            "#,
        )
        .expect("supported config should parse");

        assert_eq!(config.format.indent_width, 4);
        assert_eq!(config.format.line_endings, LineEndings::Crlf);
        assert_eq!(config.lint.line_length, 100);
        assert_eq!(config.exclude.paths, ["vendor/**", "build/**"]);
    }

    #[test]
    fn rejects_unknown_top_level_key() {
        let error = Config::parse("unknown = true").expect_err("unknown keys must fail");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_rule_ids_until_the_rule_exists() {
        let error = Config::parse(
            r#"
                [rules]
                L001 = "warning"
            "#,
        )
        .expect_err("unimplemented rule IDs must fail");

        assert!(error.to_string().contains("unknown field"));
    }
}
