//! Native diff view tests: editor input, painting, and pane behavior.
use super::super::*;
use super::diff_paint::{
    DiffColors, PaintDiffDocumentInput, PaintMarkdownDiffPreviewInput, diff_gutter_digits,
    paint_diff_document, paint_markdown_diff_preview,
};
use super::header::{HeaderAction, header_action_view};
use super::tabs::cached_terminal_theme;
use crate::app::FileActionInput;
use crate::app::painted_text;

mod editor_tests;
mod fixtures;
#[cfg(feature = "test-support")]
mod input;
mod paint_tests;
mod view_tests;

pub(crate) use fixtures::long_doc;
use fixtures::*;
#[cfg(feature = "test-support")]
use input::*;
