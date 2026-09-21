mod backend;
mod bindings;
pub mod find;
mod font;
mod keyboard;
mod theme;
mod types;
mod view;

pub use backend::settings::BackendSettings;
pub use backend::{
    BackendCommand, FindOutcome, FoundMatch, LinkTarget, PtyEvent, SearchRow, TerminalBackend,
    TerminalMode, TerminalSize,
};
pub use bindings::{Binding, BindingAction, InputKind, KeyboardBinding};
pub use find::{RowMatch, MAX_MATCHES};
pub use font::{FontSettings, TerminalFont};
pub use theme::{ColorPalette, TerminalTheme};
pub use view::{FindPaint, TerminalView};
