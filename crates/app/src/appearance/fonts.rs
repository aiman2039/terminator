//! System font choices for terminal and editor text (Settings → Appearance).
//! `fontdb` scans the system fonts and reads font files on a service pool,
//! never on the GUI thread; the GUI only installs the bytes it gets back.
use super::scale::semibold;
use super::theme::bundled_fonts;
use eframe::egui::{self, FontFamily};
use resvg::usvg::fontdb;
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    sync::{Arc, OnceLock},
};
use terminator_core::appearance::FontPrefs;
use terminator_native_edit::view::{BOLD_FAMILY, ITALIC_FAMILY};

/// Common system CJK faces; a family with one of these prefixes, or with a
/// Chinese, Japanese or Korean localized name, is offered as CJK fallback.
const CJK_HINTS: [&str; 10] = [
    "PingFang",
    "Hiragino",
    "Noto Sans CJK",
    "Noto Sans Mono CJK",
    "Noto Serif CJK",
    "Source Han",
    "Microsoft YaHei",
    "Yu Gothic",
    "Apple SD Gothic",
    "WenQuanYi",
];
const CJK_LANGUAGES: [&str; 3] = ["Chinese", "Japanese", "Korean"];

/// Font data keys of the system faces.
const TERMINAL_KEYS: [&str; 3] = ["System Mono", "System Mono Bold", "System Mono Italic"];
const CJK_KEY: &str = "System CJK";

/// Families offered in Settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FontList {
    pub monospace: Vec<String>,
    pub cjk: Vec<String>,
}

/// Font bytes read for a [`FontPrefs`] choice.
#[derive(Clone, Default)]
pub(crate) struct LoadedFonts {
    /// Regular, bold and italic faces of the terminal family.
    terminal: Option<[egui::FontData; 3]>,
    cjk: Option<egui::FontData>,
    /// Saved families that are not installed.
    pub missing: Vec<String>,
}

/// Font work that runs on a service pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    List,
    Load(FontPrefs),
}

pub(crate) enum Reply {
    Listed(Result<FontList, String>),
    Loaded(FontPrefs, Result<Box<LoadedFonts>, String>),
}

/// GUI-side font state: the installed choice and the Settings list.
#[derive(Default)]
pub(crate) struct FontState {
    /// The choice last sent to a load, or installed.
    pub requested: FontPrefs,
    pub list: Option<Result<FontList, String>>,
    pub list_requested: bool,
    /// Saved families that were not found, from the last load.
    pub missing: Vec<String>,
    pub error: Option<String>,
}

/// Service route: a newer list or load replaces the one still running.
pub(crate) fn route(op: &Op) -> (&'static str, String, terminator_core::async_service::Policy) {
    let resource = match op {
        Op::List => "list",
        Op::Load(_) => "load",
    };
    (
        "fonts",
        resource.into(),
        terminator_core::async_service::Policy::ReplaceableRead,
    )
}

/// The system font database, scanned once. Blocking: call it on a pool.
fn system() -> &'static fontdb::Database {
    static DB: OnceLock<fontdb::Database> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db
    })
}

/// Run `op` against the system fonts. Blocking: call it on a pool.
pub(crate) fn perform(op: Op) -> Reply {
    match op {
        Op::List => Reply::Listed(Ok(list(system()))),
        Op::Load(prefs) => {
            let loaded = load(system(), &prefs);
            Reply::Loaded(prefs, Ok(Box::new(loaded)))
        }
    }
}

/// Monospace and CJK family names, sorted; known CJK faces first.
pub(crate) fn list(db: &fontdb::Database) -> FontList {
    let mut monospace = BTreeSet::new();
    let mut cjk = BTreeSet::new();
    for face in db.faces() {
        let Some((name, _)) = face.families.first() else {
            continue;
        };
        if face.monospaced {
            monospace.insert(name.clone());
        }
        let localized = face
            .families
            .iter()
            .any(|(_, language)| CJK_LANGUAGES.contains(&language.primary_language()));
        if localized || cjk_hint(name) {
            cjk.insert(name.clone());
        }
    }
    let (mut cjk, rest): (Vec<_>, Vec<_>) = cjk.into_iter().partition(|name| cjk_hint(name));
    cjk.extend(rest);
    FontList {
        monospace: monospace.into_iter().collect(),
        cjk,
    }
}

fn cjk_hint(name: &str) -> bool {
    CJK_HINTS.iter().any(|hint| name.starts_with(hint))
}

/// Read the chosen faces; a family that is not installed is reported in
/// `missing` and keeps the bundled chain.
pub(crate) fn load(db: &fontdb::Database, prefs: &FontPrefs) -> LoadedFonts {
    let mut files = HashMap::new();
    let mut loaded = LoadedFonts::default();
    if let Some(family) = prefs.terminal() {
        let faces = [
            (fontdb::Weight::NORMAL, fontdb::Style::Normal),
            (fontdb::Weight::BOLD, fontdb::Style::Normal),
            (fontdb::Weight::NORMAL, fontdb::Style::Italic),
        ]
        .map(|variant| face(db, &mut files, family, variant));
        match faces {
            [Some(regular), Some(bold), Some(italic)] => {
                loaded.terminal = Some([regular, bold, italic]);
            }
            _ => loaded.missing.push(family.to_owned()),
        }
    }
    if let Some(family) = prefs.cjk() {
        let regular = (fontdb::Weight::NORMAL, fontdb::Style::Normal);
        loaded.cjk = face(db, &mut files, family, regular);
        if loaded.cjk.is_none() {
            loaded.missing.push(family.to_owned());
        }
    }
    loaded
}

/// The closest face of `family`; faces of one file share its bytes.
fn face(
    db: &fontdb::Database,
    files: &mut HashMap<PathBuf, egui::epaint::text::Blob>,
    family: &str,
    (weight, style): (fontdb::Weight, fontdb::Style),
) -> Option<egui::FontData> {
    let id = db.query(&fontdb::Query {
        families: &[fontdb::Family::Name(family)],
        weight,
        stretch: fontdb::Stretch::Normal,
        style,
    })?;
    let info = db.face(id)?;
    let path = if let fontdb::Source::File(path) = &info.source {
        Some(path.clone())
    } else {
        None
    };
    if let Some(blob) = path.as_ref().and_then(|path| files.get(path)) {
        return Some(egui::FontData::from_blob(Arc::clone(blob), info.index));
    }
    let (blob, index) = db.with_face_data(id, |data, index| {
        let blob: egui::epaint::text::Blob = Arc::new(data.to_vec());
        (blob, index)
    })?;
    if let Some(path) = path {
        files.insert(path, Arc::clone(&blob));
    }
    Some(egui::FontData::from_blob(blob, index))
}

/// The bundled chain with the loaded faces: the terminal family before
/// `JetBrains Mono`, the CJK face after the bundled fallbacks.
pub(crate) fn definitions(loaded: &LoadedFonts) -> egui::FontDefinitions {
    let mut fonts = bundled_fonts();
    if let Some(faces) = &loaded.terminal {
        let families = [
            FontFamily::Monospace,
            FontFamily::Name(BOLD_FAMILY.into()),
            FontFamily::Name(ITALIC_FAMILY.into()),
        ];
        for ((key, data), family) in TERMINAL_KEYS.iter().zip(faces).zip(families) {
            fonts
                .font_data
                .insert((*key).into(), Arc::new(data.clone()));
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, (*key).into());
        }
    }
    if let Some(data) = &loaded.cjk {
        fonts
            .font_data
            .insert(CJK_KEY.into(), Arc::new(data.clone()));
        for family in [
            FontFamily::Monospace,
            FontFamily::Name(BOLD_FAMILY.into()),
            FontFamily::Name(ITALIC_FAMILY.into()),
            FontFamily::Proportional,
            semibold(),
        ] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push(CJK_KEY.into());
        }
    }
    fonts
}
