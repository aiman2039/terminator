//! Bundled theme-tinted glyphs for Git stage verbs and diff review notes.

use eframe::egui;

use super::SourceTable;

pub(super) static REVIEW_SOURCES: SourceTable = &[
    (
        "Minus",
        egui::include_image!("../../assets/icons/minus.svg"),
    ),
    (
        "Undo2",
        egui::include_image!("../../assets/icons/undo-2.svg"),
    ),
    (
        "MessageSquarePlus",
        egui::include_image!("../../assets/icons/message-square-plus.svg"),
    ),
    ("Send", egui::include_image!("../../assets/icons/send.svg")),
];
