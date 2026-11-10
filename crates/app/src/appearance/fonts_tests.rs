use super::fonts::{definitions, list, load};
use super::theme::bundled_fonts;
use eframe::egui::FontFamily;
use resvg::usvg::fontdb;
use terminator_core::appearance::FontPrefs;
use terminator_native_edit::view::BOLD_FAMILY;

/// A database holding only the bundled fonts, so tests never scan the system.
fn bundled_db() -> fontdb::Database {
    let mut db = fontdb::Database::new();
    for bytes in [
        include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf").as_slice(),
        include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf").as_slice(),
        include_bytes!("../../assets/fonts/JetBrainsMono-Italic.ttf").as_slice(),
        include_bytes!("../../assets/fonts/Inter-Regular.ttf").as_slice(),
    ] {
        db.load_font_data(bytes.to_vec());
    }
    db
}

fn prefs(terminal: Option<&str>, cjk: Option<&str>) -> FontPrefs {
    FontPrefs {
        terminal_font: terminal.map(Into::into),
        cjk_font: cjk.map(Into::into),
    }
}

#[test]
fn missing_family_keeps_the_bundled_chain_and_is_reported() {
    let loaded = load(&bundled_db(), &prefs(Some("No Such Mono"), Some("No CJK")));
    assert_eq!(loaded.missing, ["No Such Mono", "No CJK"]);
    assert_eq!(definitions(&loaded).families, bundled_fonts().families);
}

#[test]
fn unset_choice_keeps_the_bundled_chain() {
    let loaded = load(&bundled_db(), &FontPrefs::default());
    assert!(loaded.missing.is_empty());
    assert_eq!(definitions(&loaded).families, bundled_fonts().families);
}

#[test]
fn chosen_family_goes_before_jetbrains_mono_in_every_terminal_family() {
    let loaded = load(&bundled_db(), &prefs(Some("JetBrains Mono"), None));
    assert!(loaded.missing.is_empty());
    let fonts = definitions(&loaded);
    let mono = &fonts.families[&FontFamily::Monospace];
    assert_eq!(mono[..2], ["System Mono", "JetBrains Mono"]);
    let bold = &fonts.families[&FontFamily::Name(BOLD_FAMILY.into())];
    assert_eq!(bold[0], "System Mono Bold");
}

#[test]
fn cjk_fallback_goes_last_in_terminal_and_interface_families() {
    let loaded = load(&bundled_db(), &prefs(None, Some("Inter")));
    let fonts = definitions(&loaded);
    for family in [FontFamily::Monospace, FontFamily::Proportional] {
        assert_eq!(
            fonts.families[&family].last().map(String::as_str),
            Some("System CJK")
        );
    }
}

#[test]
fn font_list_offers_only_monospace_families_for_the_terminal() {
    let fonts = list(&bundled_db());
    assert_eq!(fonts.monospace, ["JetBrains Mono"]);
    assert!(fonts.cjk.is_empty());
}
