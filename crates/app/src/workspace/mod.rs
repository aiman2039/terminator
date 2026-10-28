mod layout;
#[cfg(test)]
mod layout_tests;
mod model;
mod query;
mod views;

pub(crate) use layout::{AddAtFromInput, MovePaneToSplitInput};
pub(crate) use model::Workspace;
pub(crate) use model::WorkspaceTab;
pub(crate) use model::group_display_name;
pub(crate) use query::validate_layout;
pub(crate) use views::MoveViewToLeafInput;
