//! App integration tests, grouped by the behaviour they cover. Shared
//! fixtures and frame drivers live in `support`.

use crate::appearance;
use crate::preferences::{ExplorerSearchMode, ProjectSort};
use crate::*;
use eframe::egui::{self};
use egui_dock::{DockState, NodeIndex};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self},
    time::{Duration, Instant},
};
use support::*;
use terminator_core::*;

pub(crate) mod support;
#[cfg(feature = "test-support")]
pub(crate) use support::{CrossGlideInput, FrameGlideInput};

mod action_availability;
mod attention_inbox;
mod center_overlays;
mod daemon_recovery;
mod explorer;
mod file_open_close;
mod file_tabs;
mod float_windows;
mod git;
mod global_keys;
mod header_input_gating;
mod ide_strip;
mod layout_persistence;
mod media_player;
mod missing_folder;
mod name_prompts;
mod native_editor_panes;
mod pane_drag;
mod pane_zoom;
mod post_create;
mod project_sidebar;
mod ssh_page;
#[cfg(feature = "test-support")]
mod status_info;
mod tab_creation;
mod tab_switcher;
mod tool_windows;
mod type_ahead;
mod workspace_close;
mod workspace_tabs;
