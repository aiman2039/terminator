//! I/O-free LSP stdio core: framing, message types, request builders,
//! response parsers, UTF-16 position math, and the server command table.
//!
//! Spawning servers, owning pipes, and polling belong to the host (the
//! app manager): this module never creates processes, threads, or
//! sockets, so the crate stays standalone-publishable. The one exception
//! is [`find_server`], which stats candidate paths while probing `PATH`
//! — a pure lookup with no protocol or daemon coupling.
//!
//! Outgoing messages are built with `serde_json` (full control over the
//! minimal capability set); incoming payloads deserialize through
//! `lsp-types` and convert into the small native structs below, so the
//! view never depends on protocol types.
use std::path::{Path, PathBuf};

use serde_json::Value;
use url::Url;

use crate::highlight::Language;

mod completion;
mod edits;
mod hierarchy;
mod parsing;
mod requests;
mod server;
mod types;
mod wire;

pub use completion::*;
pub use edits::*;
pub use hierarchy::*;
pub use parsing::*;
pub use requests::*;
pub use server::*;
pub use types::*;
pub use wire::*;

#[cfg(test)]
mod tests;
