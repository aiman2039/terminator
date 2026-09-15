use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceConfig {
    pub version: u32,
    pub window: String,
    pub surface: String,
    pub hover: String,
    pub border: String,
    pub text: String,
    pub secondary: String,
    pub accent: String,
    pub selection: String,
    pub terminal_background: String,
    pub terminal_foreground: String,
    pub git_added: String,
    pub git_modified: String,
    pub git_deleted: String,
    pub git_untracked: String,
    pub git_ignored: String,
    pub status_running: String,
    pub status_waiting: String,
    pub status_failed: String,
    pub border_width: f32,
    pub pane_divider_width: f32,
    pub density: Density,
}
impl Default for AppearanceConfig {
    fn default() -> Self {
        Self {
            version: 1,
            window: "#26282C".into(),
            surface: "#191A1C".into(),
            hover: "#212326".into(),
            border: "#33353B".into(),
            text: "#D1D3D9".into(),
            secondary: "#9FA2A8".into(),
            accent: "#3871E1".into(),
            selection: "#233558".into(),
            terminal_background: "#191A1C".into(),
            terminal_foreground: "#D1D3D9".into(),
            git_added: "#81B88B".into(),
            git_modified: "#E2C08D".into(),
            git_deleted: "#C74E39".into(),
            git_untracked: "#73C991".into(),
            git_ignored: "#6E6E6E".into(),
            status_running: "#79BAA3".into(),
            status_waiting: "#E2C08D".into(),
            status_failed: "#C74E39".into(),
            border_width: 1.0,
            pane_divider_width: 8.0,
            density: Density::Comfortable,
        }
    }
}
impl AppearanceConfig {
    pub fn colors_mut(&mut self) -> Vec<(&'static str, &mut String)> {
        vec![
            ("window", &mut self.window),
            ("surface", &mut self.surface),
            ("hover", &mut self.hover),
            ("border", &mut self.border),
            ("text", &mut self.text),
            ("secondary", &mut self.secondary),
            ("accent", &mut self.accent),
            ("selection", &mut self.selection),
            ("terminal_background", &mut self.terminal_background),
            ("terminal_foreground", &mut self.terminal_foreground),
            ("git_added", &mut self.git_added),
            ("git_modified", &mut self.git_modified),
            ("git_deleted", &mut self.git_deleted),
            ("git_untracked", &mut self.git_untracked),
            ("git_ignored", &mut self.git_ignored),
            ("status_running", &mut self.status_running),
            ("status_waiting", &mut self.status_waiting),
            ("status_failed", &mut self.status_failed),
        ]
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "Unsupported appearance version {}; expected 1",
            self.version
        );
        for (name, value) in self.clone().colors_mut() {
            rgb(value).with_context(|| format!("appearance.{name}: expected #RRGGBB"))?;
        }
        ensure!(
            self.border_width.is_finite() && (0.0..=8.0).contains(&self.border_width),
            "appearance.border_width must be between 0 and 8"
        );
        ensure!(
            self.pane_divider_width.is_finite() && (1.0..=24.0).contains(&self.pane_divider_width),
            "appearance.pane_divider_width must be between 1 and 24"
        );
        Ok(())
    }
    pub fn compact(&self) -> bool {
        matches!(self.density, Density::Compact)
    }
    pub fn row_height(&self) -> f32 {
        if self.compact() { 24.0 } else { 28.0 }
    }
    pub fn high_contrast() -> Self {
        Self {
            window: "#0A0A0C".into(),
            surface: "#000000".into(),
            hover: "#1A1A1E".into(),
            border: "#5A5A60".into(),
            text: "#FFFFFF".into(),
            secondary: "#C8C8D0".into(),
            accent: "#5B9DFF".into(),
            selection: "#1A3358".into(),
            terminal_background: "#000000".into(),
            terminal_foreground: "#FFFFFF".into(),
            git_added: "#9BE09B".into(),
            git_modified: "#F0D48A".into(),
            git_deleted: "#FF6B5A".into(),
            git_untracked: "#9BE09B".into(),
            git_ignored: "#808080".into(),
            status_running: "#8EE0C0".into(),
            status_waiting: "#F0D48A".into(),
            status_failed: "#FF6B5A".into(),
            ..Default::default()
        }
    }
    pub fn colors_match(&self, other: &Self) -> bool {
        self.window == other.window
            && self.surface == other.surface
            && self.hover == other.hover
            && self.border == other.border
            && self.text == other.text
            && self.secondary == other.secondary
            && self.accent == other.accent
            && self.selection == other.selection
            && self.terminal_background == other.terminal_background
            && self.terminal_foreground == other.terminal_foreground
            && self.git_added == other.git_added
            && self.git_modified == other.git_modified
            && self.git_deleted == other.git_deleted
            && self.git_untracked == other.git_untracked
            && self.git_ignored == other.git_ignored
            && self.status_running == other.status_running
            && self.status_waiting == other.status_waiting
            && self.status_failed == other.status_failed
    }
    pub fn apply_accent(&mut self, accent: String) {
        if let Ok([r, g, b]) = rgb(&accent) {
            self.selection = format!(
                "#{:02X}{:02X}{:02X}",
                r / 3,
                g / 3,
                ((b as u16 * 2) / 5) as u8
            );
        }
        self.accent = accent;
    }
}
pub fn rgb(value: &str) -> Result<[u8; 3]> {
    ensure!(
        value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid color {value:?}"
    );
    Ok([
        u8::from_str_radix(&value[1..3], 16)?,
        u8::from_str_radix(&value[3..5], 16)?,
        u8::from_str_radix(&value[5..7], 16)?,
    ])
}
pub fn config_path(paths: &crate::Paths) -> Result<PathBuf> {
    if let Some(dir) = std::env::var_os("TERMINATOR_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let default = directories::ProjectDirs::from("dev", "terminator", "Terminator")
        .context("No data directory")?;
    if std::env::var_os("TERMINATOR_DATA_DIR").is_some() || paths.data != default.data_local_dir() {
        return Ok(paths.data.join("config.toml"));
    }
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(
            directories::BaseDirs::new()
                .context("No home directory")?
                .home_dir()
                .join(".config"),
        );
    Ok(dir.join("terminator/config.toml"))
}
#[derive(Clone, Debug)]
pub struct AppearanceFile {
    pub config: AppearanceConfig,
    pub source: String,
}
impl AppearanceFile {
    pub fn load(path: &Path) -> Result<Self> {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let doc = source
            .parse::<toml_edit::DocumentMut>()
            .context("Invalid config.toml; correct the TOML and save again")?;
        let config = if let Some(item) = doc.get("appearance") {
            toml_edit::de::from_str::<AppearanceConfig>(
                &item
                    .as_table()
                    .context("appearance must be a table")?
                    .to_string(),
            )?
        } else {
            AppearanceConfig::default()
        };
        config.validate()?;
        Ok(Self { config, source })
    }
    pub fn save(path: &Path, config: &AppearanceConfig, expected: &str) -> Result<Self> {
        config.validate()?;
        let current = Self::load(path)?;
        ensure!(
            current.source == expected,
            "Configuration changed externally; reload before applying"
        );
        let mut doc = current.source.parse::<toml_edit::DocumentMut>()?;
        if !doc.contains_key("appearance") {
            doc["appearance"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let table = doc["appearance"]
            .as_table_mut()
            .context("appearance must be a table")?;
        let values = toml_edit::ser::to_document(config)?;
        for (key, item) in values.iter() {
            let mut new = item.clone();
            if let (Some(old), Some(value)) = (
                table.get(key).and_then(|i| i.as_value()),
                new.as_value_mut(),
            ) {
                *value.decor_mut() = old.decor().clone();
            }
            table.insert(key, new);
        }
        fs::create_dir_all(
            path.parent()
                .context("Configuration has no parent directory")?,
        )?;
        let source = doc.to_string();
        crate::atomic_write(path, source.as_bytes())?;
        Ok(Self {
            config: config.clone(),
            source,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_comments_unrelated_keys_and_conflict() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        assert_eq!(
            AppearanceFile::load(&p).unwrap().config,
            AppearanceConfig::default()
        );
        assert!(!p.exists());
        let source =
            "# user comment\n[other]\nkey = 42\n[appearance]\ntext = '#FFFFFF' # keep this\n";
        fs::write(&p, source).unwrap();
        let mut file = AppearanceFile::load(&p).unwrap();
        file.config.text = "#123456".into();
        let saved = AppearanceFile::save(&p, &file.config, &file.source).unwrap();
        assert!(saved.source.contains("# keep this"));
        assert!(saved.source.contains("key = 42"));
        assert!(AppearanceFile::save(&p, &file.config, source).is_err());
    }
    #[test]
    fn invalid_external_values_are_rejected() {
        for color in ["red", "#💥abc", "#GG0000"] {
            let c = AppearanceConfig {
                text: color.into(),
                ..Default::default()
            };
            assert!(c.validate().is_err());
        }
        assert!(
            AppearanceConfig {
                border_width: f32::NAN,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn high_contrast_and_density_round_trip() {
        AppearanceConfig::high_contrast().validate().unwrap();
        let mut compact = AppearanceConfig {
            density: Density::Compact,
            ..Default::default()
        };
        assert!(compact.compact());
        assert_eq!(compact.row_height(), 24.0);
        compact.apply_accent("#3871E1".into());
        assert_eq!(compact.accent, "#3871E1");
        assert!(compact.selection.starts_with('#'));
    }
}
