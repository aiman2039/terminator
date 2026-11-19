//! Bundled theme-tinted glyphs for Git stage verbs, diff review notes,
//! the Flows tool window stripe and the Find in Files filter toggle (the
//! main table in `ui_sources` is full).

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
    (
        "Workflow",
        egui::include_image!("../../assets/icons/workflow.svg"),
    ),
    (
        "Funnel",
        egui::include_image!("../../assets/icons/funnel.svg"),
    ),
];
