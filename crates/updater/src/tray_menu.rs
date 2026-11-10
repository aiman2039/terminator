//! Linux tray model. Pure mapping from the status notice list to menu rows,
//! the tooltip text and the icon pixel format, so tests run on every OS.

use super::StatusMenuItem;

#[cfg_attr(
    not(target_os = "linux"),
    expect(dead_code, reason = "only the Linux tray renders the labels")
)]
pub(crate) const EMPTY_TITLE: &str = "No pending notifications";
#[cfg_attr(
    not(target_os = "linux"),
    expect(dead_code, reason = "only the Linux tray renders the labels")
)]
pub(crate) const SHOW_TITLE: &str = "Show Terminator";

/// One row of the tray menu, in display order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TrayRow {
    /// A pending agent notice. Picking it reports `id`.
    Notice {
        id: String,
        label: String,
    },
    /// Disabled placeholder when nothing is pending.
    Empty,
    Separator,
    /// Brings the window forward, like a click on the tray icon.
    Show,
}

/// The same rows as the macOS status menu: notices (or the disabled
/// placeholder), a separator, then Show Terminator.
pub(crate) fn tray_rows(items: &[StatusMenuItem]) -> Vec<TrayRow> {
    let mut rows: Vec<TrayRow> = if items.is_empty() {
        vec![TrayRow::Empty]
    } else {
        items
            .iter()
            .map(|item| TrayRow::Notice {
                id: item.id.clone(),
                label: menu_label(&item.title),
            })
            .collect()
    };
    rows.push(TrayRow::Separator);
    rows.push(TrayRow::Show);
    rows
}

/// D-Bus menus read `_` as an access-key marker. Double it so a notice
/// title shows its underscores.
pub(crate) fn menu_label(title: &str) -> String {
    title.replace('_', "__")
}

pub(crate) fn tray_tooltip(waiting: usize) -> String {
    match waiting {
        0 => "Terminator".into(),
        1 => "Terminator: 1 agent waiting".into(),
        n => format!("Terminator: {n} agents waiting"),
    }
}

/// RGBA8 pixels to the ARGB32 network byte order that
/// StatusNotifierItem pixmaps use. A trailing partial pixel is dropped.
pub(crate) fn rgba_to_argb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|pixel| match *pixel {
            [r, g, b, a] => [a, r, g, b],
            _ => [0, 0, 0, 0],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str) -> StatusMenuItem {
        StatusMenuItem {
            id: id.into(),
            title: title.into(),
        }
    }

    #[test]
    fn empty_tray_menu_shows_placeholder_then_show_terminator() {
        assert_eq!(
            tray_rows(&[]),
            vec![TrayRow::Empty, TrayRow::Separator, TrayRow::Show]
        );
    }

    #[test]
    fn tray_menu_lists_notices_in_order_before_show_terminator() {
        let rows = tray_rows(&[item("a", "build — Waiting: ok"), item("b", "my_task")]);
        assert_eq!(
            rows,
            vec![
                TrayRow::Notice {
                    id: "a".into(),
                    label: "build — Waiting: ok".into()
                },
                TrayRow::Notice {
                    id: "b".into(),
                    label: "my__task".into()
                },
                TrayRow::Separator,
                TrayRow::Show,
            ]
        );
    }

    #[test]
    fn tray_tooltip_counts_waiting_agents() {
        assert_eq!(tray_tooltip(0), "Terminator");
        assert_eq!(tray_tooltip(1), "Terminator: 1 agent waiting");
        assert_eq!(tray_tooltip(7), "Terminator: 7 agents waiting");
    }

    #[test]
    fn tray_icon_pixels_move_alpha_first() {
        assert_eq!(
            rgba_to_argb(&[1, 2, 3, 4, 5, 6, 7, 8, 9]),
            vec![4, 1, 2, 3, 8, 5, 6, 7]
        );
    }
}
