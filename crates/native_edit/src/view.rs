//! Embeddable native source view over [`crate::doc::Buffer`].
mod keys;
mod render;
#[cfg(test)]
mod view_tests;
pub use keys::{
    IdeAction, command_prompt, ctrl_h_is_motion, ctrl_v_is_block, echo_of_handled_key, egui_key,
    ide_shortcut, mode_color, mode_name, source_focus_id,
};
pub use render::{
    DiagnosticMark, SourceOptions, SourceOutcome, char_display_width, col_at_display_x, display_x,
    show_source, status_line,
};
