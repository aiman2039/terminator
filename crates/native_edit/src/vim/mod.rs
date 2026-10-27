//! Minimal vim-modal engine over [`crate::doc::Buffer`].
mod carets;
mod cursor;
mod helpers;
mod ide;
mod insert;
mod modal;
mod mode;
mod motions;
mod mouse;
mod normal;
mod operators;
mod record;
mod repeat;
mod state;
#[cfg(test)]
mod vim_tests;
mod words;
pub use carets::Caret;
pub use ide::IdeMotion;
pub use mode::{Cursor, Effect, Key, ModalEngine, Mode};
pub use state::{VimEngine, VimrcOptions, parse_vimrc};
