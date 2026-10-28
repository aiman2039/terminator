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

pub(super) mod support;
#[cfg(feature = "test-support")]
pub(crate) use support::{CrossGlideInput, FrameGlideInput};

mod attention_inbox;
mod center_overlays;
mod daemon_recovery;
mod explorer;
mod file_open_close;
mod float_windows;
mod git_panel;
mod header_input_gating;
mod ide_strip;
mod layout_persistence;
mod media_player;
mod name_prompts;
mod native_editor_panes;
mod pane_drag;
mod project_sidebar;
#[cfg(feature = "test-support")]
mod status_info;
mod tab_creation;
mod workspace_close;
mod workspace_tabs;
