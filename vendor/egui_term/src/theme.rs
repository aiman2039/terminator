use alacritty_terminal::vte::ansi::{self, NamedColor};
use egui::Color32;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub struct ColorPalette {
    pub foreground: String,
    pub background: String,
    pub black: String,
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub blue: String,
    pub magenta: String,
    pub cyan: String,
    pub white: String,
    pub bright_black: String,
    pub bright_red: String,
    pub bright_green: String,
    pub bright_yellow: String,
    pub bright_blue: String,
    pub bright_magenta: String,
    pub bright_cyan: String,
    pub bright_white: String,
    pub bright_foreground: Option<String>,
    pub dim_foreground: String,
    pub dim_black: String,
    pub dim_red: String,
    pub dim_green: String,
    pub dim_yellow: String,
    pub dim_blue: String,
    pub dim_magenta: String,
    pub dim_cyan: String,
    pub dim_white: String,
}

impl Default for ColorPalette {
    fn default() -> Self {
        Self {
            foreground: String::from("#d1d3d9"),
            background: String::from("#191a1c"),
            black: String::from("#191a1c"),
            red: String::from("#ac4242"),
            green: String::from("#90a959"),
            yellow: String::from("#f4bf75"),
            blue: String::from("#6a9fb5"),
            magenta: String::from("#aa759f"),
            cyan: String::from("#75b5aa"),
            white: String::from("#d1d3d9"),
            bright_black: String::from("#6b6b6b"),
            bright_red: String::from("#c55555"),
            bright_green: String::from("#aac474"),
            bright_yellow: String::from("#feca88"),
            bright_blue: String::from("#82b8c8"),
            bright_magenta: String::from("#c28cb8"),
            bright_cyan: String::from("#93d3c3"),
            bright_white: String::from("#f8f8f8"),
            bright_foreground: None,
            dim_foreground: String::from("#828482"),
            dim_black: String::from("#0f0f0f"),
            dim_red: String::from("#712b2b"),
            dim_green: String::from("#5f6f3a"),
            dim_yellow: String::from("#a17e4d"),
            dim_blue: String::from("#456877"),
            dim_magenta: String::from("#704d68"),
            dim_cyan: String::from("#4d7770"),
            dim_white: String::from("#8e8e8e"),
        }
    }
}

const COLOR_COUNT: usize = NamedColor::DimForeground as usize + 1;

/// Immutable, parsed colors shared by frame-local terminal widgets.
#[derive(Debug, Clone)]
pub struct TerminalTheme {
    colors: Arc<[Color32; COLOR_COUNT]>,
}

impl Default for TerminalTheme {
    fn default() -> Self {
        static DEFAULT: OnceLock<TerminalTheme> = OnceLock::new();
        DEFAULT.get_or_init(|| Self::new(Box::default())).clone()
    }
}

impl TerminalTheme {
    pub fn new(palette: Box<ColorPalette>) -> Self {
        let parse =
            |value: &str| hex_to_color(value).unwrap_or_else(|_| panic!("invalid color {}", value));
        let mut colors = [parse(&palette.background); COLOR_COUNT];
        colors[NamedColor::Black as usize] = parse(&palette.black);
        colors[NamedColor::Red as usize] = parse(&palette.red);
        colors[NamedColor::Green as usize] = parse(&palette.green);
        colors[NamedColor::Yellow as usize] = parse(&palette.yellow);
        colors[NamedColor::Blue as usize] = parse(&palette.blue);
        colors[NamedColor::Magenta as usize] = parse(&palette.magenta);
        colors[NamedColor::Cyan as usize] = parse(&palette.cyan);
        colors[NamedColor::White as usize] = parse(&palette.white);
        colors[NamedColor::BrightBlack as usize] = parse(&palette.bright_black);
        colors[NamedColor::BrightRed as usize] = parse(&palette.bright_red);
        colors[NamedColor::BrightGreen as usize] = parse(&palette.bright_green);
        colors[NamedColor::BrightYellow as usize] = parse(&palette.bright_yellow);
        colors[NamedColor::BrightBlue as usize] = parse(&palette.bright_blue);
        colors[NamedColor::BrightMagenta as usize] = parse(&palette.bright_magenta);
        colors[NamedColor::BrightCyan as usize] = parse(&palette.bright_cyan);
        colors[NamedColor::BrightWhite as usize] = parse(&palette.bright_white);
        colors[NamedColor::Foreground as usize] = parse(&palette.foreground);
        colors[NamedColor::Background as usize] = parse(&palette.background);
        colors[NamedColor::DimBlack as usize] = parse(&palette.dim_black);
        colors[NamedColor::DimRed as usize] = parse(&palette.dim_red);
        colors[NamedColor::DimGreen as usize] = parse(&palette.dim_green);
        colors[NamedColor::DimYellow as usize] = parse(&palette.dim_yellow);
        colors[NamedColor::DimBlue as usize] = parse(&palette.dim_blue);
        colors[NamedColor::DimMagenta as usize] = parse(&palette.dim_magenta);
        colors[NamedColor::DimCyan as usize] = parse(&palette.dim_cyan);
        colors[NamedColor::DimWhite as usize] = parse(&palette.dim_white);
        colors[NamedColor::DimForeground as usize] = parse(&palette.dim_foreground);
        colors[NamedColor::BrightForeground as usize] = palette
            .bright_foreground
            .as_deref()
            .map(parse)
            .unwrap_or(colors[NamedColor::Foreground as usize]);
        for index in 16..232 {
            let n = index - 16;
            let component = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
            colors[index] =
                Color32::from_rgb(component(n / 36), component(n / 6 % 6), component(n % 6));
        }
        for index in 232..256 {
            colors[index] = Color32::from_gray(((index - 232) * 10 + 8) as u8);
        }
        Self {
            colors: Arc::new(colors),
        }
    }

    pub fn get_color(&self, color: ansi::Color) -> Color32 {
        match color {
            ansi::Color::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
            ansi::Color::Indexed(index) => self.colors[index as usize],
            ansi::Color::Named(name) => self.colors[name as usize],
        }
    }
}

fn hex_to_color(hex: &str) -> anyhow::Result<Color32> {
    if hex.len() != 7 {
        return Err(anyhow::format_err!("input string is in non valid format"));
    }

    let r = u8::from_str_radix(&hex[1..3], 16)?;
    let g = u8::from_str_radix(&hex[3..5], 16)?;
    let b = u8::from_str_radix(&hex[5..7], 16)?;

    Ok(Color32::from_rgb(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_cube_and_grayscale_keep_their_colors() {
        let theme = TerminalTheme::default();
        for index in 16u8..=255 {
            let expected = if index < 232 {
                let n = usize::from(index) - 16;
                let levels = [0, 95, 135, 175, 215, 255];
                Color32::from_rgb(levels[n / 36], levels[n / 6 % 6], levels[n % 6])
            } else {
                Color32::from_gray((index - 232) * 10 + 8)
            };
            assert_eq!(theme.get_color(ansi::Color::Indexed(index)), expected);
        }
    }

    #[test]
    fn custom_colors_and_bright_foreground_fallback_survive_caching() {
        let palette = ColorPalette {
            foreground: "#123456".into(),
            background: "#654321".into(),
            red: "#abcdef".into(),
            ..Default::default()
        };
        let theme = TerminalTheme::new(Box::new(palette.clone()));
        assert_eq!(
            theme.get_color(ansi::Color::Named(NamedColor::Foreground)),
            Color32::from_rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Named(NamedColor::BrightForeground)),
            theme.get_color(ansi::Color::Named(NamedColor::Foreground))
        );
        assert_eq!(
            theme.get_color(ansi::Color::Named(NamedColor::Cursor)),
            Color32::from_rgb(0x65, 0x43, 0x21)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(1)),
            Color32::from_rgb(0xab, 0xcd, 0xef)
        );
        assert_eq!(
            theme.get_color(ansi::Color::Indexed(1)),
            theme.get_color(ansi::Color::Named(NamedColor::Red))
        );
        let bright = TerminalTheme::new(Box::new(ColorPalette {
            bright_foreground: Some("#ffffff".into()),
            ..palette
        }));
        assert_eq!(
            bright.get_color(ansi::Color::Named(NamedColor::BrightForeground)),
            Color32::WHITE
        );
        assert!(Arc::ptr_eq(&theme.colors, &theme.clone().colors));
        assert!(Arc::ptr_eq(
            &TerminalTheme::default().colors,
            &TerminalTheme::default().colors
        ));
    }
}
