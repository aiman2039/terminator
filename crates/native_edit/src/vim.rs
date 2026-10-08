//! Minimal vim-modal engine over [`crate::doc::Buffer`].
mod cursor;
mod helpers;
mod insert;
mod modal;
mod mode;
mod motions;
mod normal;
mod operators;
mod record;
mod repeat;
mod state;
#[cfg(test)]
mod vim_tests;
mod words;
pub use mode::{Cursor, Effect, Key, ModalEngine, Mode};
pub use state::{VimEngine, VimrcOptions, parse_vimrc};
