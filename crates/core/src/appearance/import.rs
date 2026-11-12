//! Terminal theme import: Ghostty and kitty color files and iTerm2
//! `.itermcolors` files, mapped to the appearance color tokens. An
//! import only fills a draft; nothing is saved until the user applies it.
use super::{AppearanceConfig, rgb};

type Rgb = [u8; 3];

/// Colors read from a terminal theme file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: Option<Rgb>,
    pub foreground: Option<Rgb>,
    pub cursor: Option<Rgb>,
    pub selection: Option<Rgb>,
    /// ANSI colors 0..=15.
    pub palette: [Option<Rgb>; 16],
}

/// Why a theme file could not be imported.
#[derive(Debug, thiserror::Error)]
pub enum ThemeImportError {
    #[error("No background and foreground colors found; use a Ghostty, kitty or iTerm2 theme")]
    MissingColors,
    #[error("Invalid iTerm2 color file: {0}")]
    Plist(String),
}

/// `#RRGGBB` or `RRGGBB` (Ghostty writes both).
fn hex(value: &str) -> Option<Rgb> {
    let value = value.trim().trim_matches('"');
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 {
        return None;
    }
    rgb(&format!("#{value}")).ok()
}

impl TerminalColors {
    /// Read a Ghostty (`key = value`), kitty (`key value`) or iTerm2
    /// (XML property list) theme.
    pub fn parse(text: &str) -> Result<Self, ThemeImportError> {
        let colors = if text.contains("<plist") {
            Self::parse_iterm(text)?
        } else {
            Self::parse_lines(text)
        };
        if colors.background.is_none() || colors.foreground.is_none() {
            return Err(ThemeImportError::MissingColors);
        }
        Ok(colors)
    }

    fn parse_lines(text: &str) -> Self {
        let mut colors = Self::default();
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') || line.starts_with("//") {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                // kitty values never hold '=' before the color.
                Some((key, value)) if !key.trim().contains(' ') => (key.trim(), value.trim()),
                _ => match line.split_once(char::is_whitespace) {
                    Some((key, value)) => (key.trim(), value.trim()),
                    None => continue,
                },
            };
            colors.set(key, value);
        }
        colors
    }

    fn set(&mut self, key: &str, value: &str) {
        match key {
            "background" => self.background = hex(value),
            "foreground" => self.foreground = hex(value),
            "cursor" | "cursor-color" => self.cursor = hex(value),
            "selection_background" | "selection-background" => self.selection = hex(value),
            // Ghostty: `palette = 4=#5e81ac`.
            "palette" => {
                if let Some((index, color)) = value.split_once('=') {
                    self.set_ansi(index.trim(), color);
                }
            }
            // kitty: `color4 #5e81ac`.
            _ => {
                if let Some(index) = key.strip_prefix("color") {
                    self.set_ansi(index, value);
                }
            }
        }
    }

    fn set_ansi(&mut self, index: &str, value: &str) {
        if let Ok(index) = index.parse::<usize>()
            && let Some(slot) = self.palette.get_mut(index)
        {
            *slot = hex(value);
        }
    }

    fn parse_iterm(text: &str) -> Result<Self, ThemeImportError> {
        let value = plist::Value::from_reader_xml(text.as_bytes())
            .map_err(|error| ThemeImportError::Plist(error.to_string()))?;
        let dict = value
            .as_dictionary()
            .ok_or_else(|| ThemeImportError::Plist("not a dictionary".into()))?;
        let color = |key: &str| -> Option<Rgb> {
            let entry = dict.get(key)?.as_dictionary()?;
            let channel = |name: &str| -> Option<u8> {
                let value = entry.get(name)?.as_real()?.clamp(0.0, 1.0);
                // Rounded 0..=255; the clamp keeps the cast in range.
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped to 0..=1, so the rounded value fits u8"
                )]
                let byte = (value * 255.0).round() as u8;
                Some(byte)
            };
            Some([
                channel("Red Component")?,
                channel("Green Component")?,
                channel("Blue Component")?,
            ])
        };
        let mut colors = Self {
            background: color("Background Color"),
            foreground: color("Foreground Color"),
            cursor: color("Cursor Color"),
            selection: color("Selection Color"),
            ..Self::default()
        };
        for (index, slot) in colors.palette.iter_mut().enumerate() {
            *slot = color(&format!("Ansi {index} Color"));
        }
        Ok(colors)
    }
}

fn to_hex([r, g, b]: Rgb) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// `a` moved `percent` of the way to `b`.
fn mix(a: Rgb, b: Rgb, percent: u16) -> Rgb {
    let channel = |a: u8, b: u8| {
        let value = u16::from(a)
            .saturating_mul(100_u16.saturating_sub(percent))
            .saturating_add(u16::from(b).saturating_mul(percent))
            .saturating_add(50)
            / 100;
        u8::try_from(value).unwrap_or(u8::MAX)
    };
    [
        channel(a[0], b[0]),
        channel(a[1], b[1]),
        channel(a[2], b[2]),
    ]
}

impl AppearanceConfig {
    /// This configuration with its colors taken from a terminal theme.
    /// Sizes, density, fonts and editor settings stay as they are.
    #[must_use]
    pub fn with_terminal_colors(&self, colors: &TerminalColors) -> Self {
        let mut next = self.clone();
        let (Some(bg), Some(fg)) = (colors.background, colors.foreground) else {
            return next;
        };
        let ansi = |index: usize, fallback: Rgb| {
            colors
                .palette
                .get(index)
                .copied()
                .flatten()
                .unwrap_or(fallback)
        };
        let red = ansi(1, [0xE0, 0x6C, 0x75]);
        let green = ansi(2, [0x98, 0xC3, 0x79]);
        let yellow = ansi(3, [0xE5, 0xC0, 0x7B]);
        let blue = ansi(4, [0x61, 0xAF, 0xEF]);
        let accent = colors.cursor.filter(|cursor| *cursor != fg).unwrap_or(blue);
        next.window = to_hex(bg);
        next.surface = to_hex(mix(bg, fg, 5));
        next.hover = to_hex(mix(bg, fg, 12));
        next.border = to_hex(mix(bg, fg, 16));
        next.text = to_hex(fg);
        next.secondary = to_hex(mix(fg, bg, 35));
        next.accent = to_hex(accent);
        next.primary = None;
        next.selection = to_hex(colors.selection.unwrap_or_else(|| mix(bg, accent, 30)));
        next.terminal_background = to_hex(bg);
        next.terminal_foreground = to_hex(fg);
        next.git_added = to_hex(green);
        next.git_modified = to_hex(blue);
        next.git_deleted = to_hex(red);
        next.git_untracked = to_hex(ansi(10, green));
        next.git_ignored = to_hex(ansi(8, mix(fg, bg, 50)));
        next.status_running = to_hex(green);
        next.status_waiting = to_hex(yellow);
        next.status_failed = to_hex(red);
        next
    }
}
