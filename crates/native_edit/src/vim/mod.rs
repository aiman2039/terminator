//! Minimal vim-modal engine over [`crate::doc::Buffer`].
mod block;
mod carets;
mod command;
mod cursor;
mod edit;
mod ex;
mod helpers;
mod ide;
mod input;
mod insert;
mod modal;
mod mode;
mod motion;
mod motions;
mod mouse;
mod normal;
mod operators;
#[cfg(test)]
mod paste_tests;
mod record;
mod register;
mod repeat;
mod search;
mod state;
mod substitute;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vim_tests;
mod visual;
mod words;
pub use carets::Caret;
pub use ide::{CaretOffsets, IdeMotion, SnippetMark};
pub use input::mapping::{MapMode, Mapping, Mappings};
pub use mode::{Cursor, Effect, Key, ModalEngine, Mode};
pub use state::{ViewScroll, VimEngine, VimrcOptions, parse_vimrc};
