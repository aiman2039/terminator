//! `IntelliJ` project-tree folder colors: blue source roots, green test
//! roots on a faint green row, orange excluded folders on a faint orange
//! row. Values from the `JetBrains` expUI node icons and `FileColor` tokens.
use crate::icons::FolderRole;
use eframe::egui::Color32;

/// Folder glyph tint and optional row background for one role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderTint {
    pub icon: Option<Color32>,
    pub fill: Option<Color32>,
}

/// Row background alpha: a faint tint that keeps the label legible.
const FILL_ALPHA: u8 = 30;

fn tinted(icon: Color32) -> Option<Color32> {
    let [r, g, b, _] = icon.to_array();
    Some(Color32::from_rgba_unmultiplied(r, g, b, FILL_ALPHA))
}

/// Colors for a folder `role`; plain folders keep the theme icon color.
pub fn folder_tint(role: FolderRole, light: bool) -> FolderTint {
    let (source, test, excluded) = if light {
        (
            Color32::from_rgb(0x35, 0x74, 0xF0),
            Color32::from_rgb(0x20, 0x8A, 0x3C),
            Color32::from_rgb(0xE6, 0x6D, 0x17),
        )
    } else {
        // `sourceRoot_dark.svg`, `testRoot_dark.svg`, `excludeRoot_dark.svg`.
        (
            Color32::from_rgb(0x54, 0x8A, 0xF7),
            Color32::from_rgb(0x57, 0x96, 0x5C),
            Color32::from_rgb(0xC7, 0x7D, 0x55),
        )
    };
    match role {
        FolderRole::Source => FolderTint {
            icon: Some(source),
            fill: None,
        },
        FolderRole::Test => FolderTint {
            icon: Some(test),
            fill: tinted(test),
        },
        FolderRole::Excluded => FolderTint {
            icon: Some(excluded),
            fill: tinted(excluded),
        },
        FolderRole::Plain => FolderTint {
            icon: None,
            fill: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_roles_take_the_intellij_project_tree_colors() {
        let source = folder_tint(FolderRole::Source, false);
        assert_eq!(source.icon, Some(Color32::from_rgb(0x54, 0x8A, 0xF7)));
        assert_eq!(source.fill, None);
        let test = folder_tint(FolderRole::Test, false);
        assert_eq!(test.icon, Some(Color32::from_rgb(0x57, 0x96, 0x5C)));
        assert!(test.fill.is_some_and(|fill| fill.a() == FILL_ALPHA));
        let excluded = folder_tint(FolderRole::Excluded, false);
        assert_eq!(excluded.icon, Some(Color32::from_rgb(0xC7, 0x7D, 0x55)));
        assert!(excluded.fill.is_some());
        assert_eq!(
            folder_tint(FolderRole::Plain, false),
            FolderTint {
                icon: None,
                fill: None
            }
        );
        assert_ne!(
            folder_tint(FolderRole::Source, true).icon,
            source.icon,
            "light themes use the light icon set"
        );
    }
}
