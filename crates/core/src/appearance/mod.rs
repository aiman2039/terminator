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
    /// Fill of the main action button. Unset uses `accent`. A theme file
    /// without the key stays unset, whatever the preset sets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<String>,
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
    #[serde(flatten)]
    pub editor: crate::editor_prefs::EditorPrefs,
    #[serde(flatten)]
    pub fonts: FontPrefs,
}

/// System font choices for terminal and editor text. `None` keeps the
/// bundled `JetBrains Mono` chain; a missing key is never written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FontPrefs {
    /// Monospace family placed before `JetBrains Mono`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_font: Option<String>,
    /// Family appended to the fallback chain for CJK text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cjk_font: Option<String>,
}

impl FontPrefs {
    /// Optional keys; saving removes each one that is unset.
    pub const KEYS: [&'static str; 2] = ["terminal_font", "cjk_font"];

    /// The terminal family, or `None` when unset or blank.
    #[must_use]
    pub fn terminal(&self) -> Option<&str> {
        Self::named(self.terminal_font.as_deref())
    }

    /// The CJK fallback family, or `None` when unset or blank.
    #[must_use]
    pub fn cjk(&self) -> Option<&str> {
        Self::named(self.cjk_font.as_deref())
    }

    fn named(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|name| !name.is_empty())
    }
}
impl Default for AppearanceConfig {
    fn default() -> Self {
        Self::carbon()
    }
}
impl AppearanceConfig {
    /// Optional key; saving removes it when unset.
    pub const PRIMARY_KEY: &'static str = "primary";

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
    pub fn validate(&self) -> Result<(), AppearanceError> {
        if self.version != 1 {
            return Err(AppearanceError::UnsupportedVersion {
                version: self.version,
            });
        }
        for (name, value) in self.clone().colors_mut() {
            rgb(value).map_err(|source| AppearanceError::InvalidColorField { name, source })?;
        }
        if let Some(primary) = &self.primary {
            rgb(primary).map_err(|source| AppearanceError::InvalidColorField {
                name: "primary",
                source,
            })?;
        }
        if !(self.border_width.is_finite() && (0.0..=8.0).contains(&self.border_width)) {
            return Err(AppearanceError::BorderWidth {
                width: self.border_width,
            });
        }
        if !(self.pane_divider_width.is_finite() && (1.0..=24.0).contains(&self.pane_divider_width))
        {
            return Err(AppearanceError::PaneDividerWidth {
                width: self.pane_divider_width,
            });
        }
        Ok(())
    }
    #[must_use]
    pub fn compact(&self) -> bool {
        matches!(self.density, Density::Compact)
    }
    #[must_use]
    pub fn row_height(&self) -> f32 {
        if self.compact() { 24.0 } else { 28.0 }
    }
    /// Whether the window color is light, so chrome should use dark glyphs.
    #[must_use]
    pub fn is_light(&self) -> bool {
        rgb(&self.window).is_ok_and(|[r, g, b]| {
            // Rec. 601 luma on 0..=255 channels.
            0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 128.0
        })
    }
    #[must_use]
    pub fn colors_match(&self, other: &Self) -> bool {
        self.window == other.window
            && self.surface == other.surface
            && self.hover == other.hover
            && self.border == other.border
            && self.text == other.text
            && self.secondary == other.secondary
            && self.accent == other.accent
            && self.primary == other.primary
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
    /// The main action fill: `primary`, else `accent`.
    #[must_use]
    pub fn primary_color(&self) -> &str {
        self.primary.as_deref().unwrap_or(&self.accent)
    }
    pub fn apply_accent(&mut self, accent: String) {
        if let Ok([r, g, b]) = rgb(&accent) {
            self.selection = format!(
                "#{:02X}{:02X}{:02X}",
                r / 3,
                g / 3,
                u8::try_from(u16::from(b).saturating_mul(2).checked_div(5).unwrap_or(0),)
                    .unwrap_or(0)
            );
        }
        self.accent = accent;
    }
}
pub fn rgb(value: &str) -> Result<[u8; 3], ColorError> {
    let invalid = || ColorError::Invalid {
        value: value.to_owned(),
    };
    let Some(body) = value.get(1..) else {
        return Err(invalid());
    };
    if !(value.len() == 7 && value.starts_with('#') && body.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid());
    }
    let hex = |start: usize| -> Result<u8, ColorError> {
        let end = start.saturating_add(2);
        let part = value.get(start..end).ok_or_else(|| ColorError::Truncated {
            value: value.to_owned(),
        })?;
        Ok(u8::from_str_radix(part, 16)?)
    };
    Ok([hex(1)?, hex(3)?, hex(5)?])
}
pub fn config_path(paths: &crate::Paths) -> Result<PathBuf, AppearanceError> {
    if let Some(dir) = std::env::var_os("TERMINATOR_CONFIG_DIR") {
        return Ok(PathBuf::from(dir).join("config.toml"));
    }
    let default = directories::ProjectDirs::from("dev", "terminator", "Terminator")
        .ok_or(AppearanceError::NoDataDirectory)?;
    if std::env::var_os("TERMINATOR_DATA_DIR").is_some() || paths.data != default.data_local_dir() {
        return Ok(paths.data.join("config.toml"));
    }
    let dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(
            directories::BaseDirs::new()
                .ok_or(AppearanceError::NoHomeDirectory)?
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
    pub fn load(path: &Path) -> Result<Self, AppearanceError> {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let doc = source.parse::<toml_edit::DocumentMut>().map_err(|source| {
            AppearanceError::InvalidToml {
                path: path.to_path_buf(),
                source,
            }
        })?;
        let config = if let Some(item) = doc.get("appearance") {
            toml_edit::de::from_str::<AppearanceConfig>(
                &item
                    .as_table()
                    .ok_or(AppearanceError::NotATable)?
                    .to_string(),
            )?
        } else {
            AppearanceConfig::default()
        };
        config.validate()?;
        Ok(Self { config, source })
    }
    pub fn save(
        path: &Path,
        config: &AppearanceConfig,
        expected: &str,
    ) -> Result<Self, AppearanceError> {
        config.validate()?;
        let current = Self::load(path)?;
        if current.source != expected {
            return Err(AppearanceError::ChangedExternally {
                path: path.to_path_buf(),
            });
        }
        let mut doc = current.source.parse::<toml_edit::DocumentMut>()?;
        if !doc.contains_key("appearance") {
            doc.insert(
                "appearance",
                toml_edit::Item::Table(toml_edit::Table::new()),
            );
        }
        let table = doc
            .get_mut("appearance")
            .and_then(toml_edit::Item::as_table_mut)
            .ok_or(AppearanceError::NotATable)?;
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
        for key in FontPrefs::KEYS
            .iter()
            .chain(&[AppearanceConfig::PRIMARY_KEY])
        {
            if !values.contains_key(key) {
                table.remove(key);
            }
        }
        fs::create_dir_all(path.parent().ok_or_else(|| AppearanceError::NoParent {
            path: path.to_path_buf(),
        })?)?;
        let source = doc.to_string();
        crate::atomic_replace(path, source.as_bytes())?;
        Ok(Self {
            config: config.clone(),
            source,
        })
    }
}
/// Why a `#RRGGBB` color string is invalid.
#[derive(Debug, thiserror::Error)]
pub enum ColorError {
    #[error("Invalid color {value:?}")]
    Invalid { value: String },
    #[error("Invalid color")]
    Truncated { value: String },
    #[error(transparent)]
    Hex(#[from] std::num::ParseIntError),
}

/// Why the appearance configuration could not be validated, loaded, or saved.
#[derive(Debug, thiserror::Error)]
pub enum AppearanceError {
    #[error("Unsupported appearance version {version}; expected 1")]
    UnsupportedVersion { version: u32 },
    #[error("appearance.{name}: expected #RRGGBB")]
    InvalidColorField {
        name: &'static str,
        #[source]
        source: ColorError,
    },
    #[error("appearance.border_width must be between 0 and 8")]
    BorderWidth { width: f32 },
    #[error("appearance.pane_divider_width must be between 1 and 24")]
    PaneDividerWidth { width: f32 },
    #[error("No data directory")]
    NoDataDirectory,
    #[error("No home directory")]
    NoHomeDirectory,
    #[error("Invalid config.toml; correct the TOML and save again")]
    InvalidToml {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },
    #[error("appearance must be a table")]
    NotATable,
    #[error("Configuration changed externally; reload before applying")]
    ChangedExternally { path: PathBuf },
    #[error("Configuration has no parent directory")]
    NoParent { path: PathBuf },
    #[error(transparent)]
    Toml(#[from] toml_edit::TomlError),
    #[error(transparent)]
    Deserialize(#[from] toml_edit::de::Error),
    #[error(transparent)]
    Serialize(#[from] toml_edit::ser::Error),
    #[error(transparent)]
    Write(#[from] crate::AtomicWriteError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

mod presets;
#[cfg(test)]
mod tests;
