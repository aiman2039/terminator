//! Native editing core: buffer plus minimal vim-modal engine.
#![forbid(unsafe_code)]
//!
//! All consumers program against [`doc::Buffer`] and [`vim::ModalEngine`]:
//! `ropey` replaces [`doc::Doc`] and `hjkl` replaces [`vim::VimEngine`
//! without touching callers once the sandbox allows new crates.

pub mod blocks;
pub mod clipboard;
pub mod doc;
pub mod finder;
pub mod highlight;
pub mod lsp;
pub mod treesitter;
pub mod view;
pub mod vim;
