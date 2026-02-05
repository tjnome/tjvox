use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;
use tracing::{debug, info, warn};

pub struct ReplacementEngine {
    rules: Vec<(Regex, String)>,
}

impl ReplacementEngine {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            info!(
                "Replacements file not found at {:?}, creating default",
                path
            );
            Self::create_default(path)?;
        }

        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read replacements file: {:?}", path))?;

        let table: toml::Value = toml::from_str(&content)
            .with_context(|| format!("Failed to parse replacements file: {:?}", path))?;

        let replacements = table
            .get("replacements")
            .and_then(|v| v.as_table())
            .cloned()
            .unwrap_or_default();

        let mut rules = Vec::new();
        for (key, value) in &replacements {
            if let Some(replacement) = value.as_str() {
                let pattern = format!(r"(?i)\b{}\b", regex::escape(key));
                match Regex::new(&pattern) {
                    Ok(re) => {
                        debug!("Loaded replacement rule: {:?} -> {:?}", key, replacement);
                        rules.push((re, replacement.to_string()));
                    }
                    Err(e) => {
                        warn!("Invalid replacement pattern for {:?}: {}", key, e);
                    }
                }
            }
        }

        info!("Loaded {} replacement rules from {:?}", rules.len(), path);
        Ok(Self { rules })
    }

    pub fn apply(&self, text: &str) -> String {
        let mut result = text.to_string();
        for (pattern, replacement) in &self.rules {
            result = pattern
                .replace_all(&result, replacement.as_str())
                .to_string();
        }
        result
    }

    fn create_default(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let default = r#"[replacements]
"period" = "."
"comma" = ","
"question mark" = "?"
"exclamation mark" = "!"
"new line" = "\n"
"new paragraph" = "\n\n"
"#;

        std::fs::write(path, default)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_replacement_engine_apply() {
        let rules = vec![
            (Regex::new(r"(?i)\bperiod\b").unwrap(), ".".to_string()),
            (Regex::new(r"(?i)\bcomma\b").unwrap(), ",".to_string()),
        ];
        let engine = ReplacementEngine { rules };

        let result = engine.apply("Hello period world comma");
        assert_eq!(result, "Hello . world ,");
    }

    #[test]
    fn test_replacement_engine_case_insensitive() {
        let rules = vec![(Regex::new(r"(?i)\bperiod\b").unwrap(), ".".to_string())];
        let engine = ReplacementEngine { rules };

        let result = engine.apply("Hello PERIOD world Period");
        assert_eq!(result, "Hello . world .");
    }

    #[test]
    fn test_replacement_engine_whole_word_only() {
        let rules = vec![(Regex::new(r"(?i)\bperiod\b").unwrap(), ".".to_string())];
        let engine = ReplacementEngine { rules };

        // "periods" should not be replaced because of word boundary
        // Only "period" should be replaced
        let result = engine.apply("There are many periods but not period");
        assert_eq!(result, "There are many periods but not .");
    }

    #[test]
    fn test_replacement_engine_special_chars() {
        let rules = vec![
            (Regex::new(r"(?i)\bnew line\b").unwrap(), "\n".to_string()),
            (
                Regex::new(r"(?i)\bnew paragraph\b").unwrap(),
                "\n\n".to_string(),
            ),
        ];
        let engine = ReplacementEngine { rules };

        let result = engine.apply("Line one new line Line two new paragraph Line three");
        assert_eq!(result, "Line one \n Line two \n\n Line three");
    }

    #[test]
    fn test_replacement_engine_empty() {
        let engine = ReplacementEngine { rules: vec![] };
        let text = "Hello world";
        let result = engine.apply(text);
        assert_eq!(result, text);
    }

    #[test]
    fn test_replacement_engine_load_and_apply() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("replacements.toml");

        // Create test replacements file
        let content = r#"[replacements]
"test" = "replaced"
"hello" = "hi"
"#;
        std::fs::write(&path, content).unwrap();

        let engine = ReplacementEngine::load(&path).unwrap();
        let result = engine.apply("test hello world");
        assert_eq!(result, "replaced hi world");
    }

    #[test]
    fn test_replacement_engine_creates_default() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("replacements.toml");

        // Should create default file
        let engine = ReplacementEngine::load(&path).unwrap();
        assert!(path.exists());

        // Verify default works
        let result = engine.apply("Hello period world comma");
        assert_eq!(result, "Hello . world ,");
    }
}
