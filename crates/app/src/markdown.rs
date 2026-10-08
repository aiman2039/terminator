mod html;
#[cfg(test)]
mod markdown_tests;
mod previews;
mod source;
pub(crate) use html::Link;
pub(crate) use previews::Previews;
pub(crate) use source::{Mode, Source, available, supported};
