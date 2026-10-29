use super::source::{Revision, Snapshot};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, TextMergeWithOffset};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use url::Url;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Link {
    File(PathBuf),
    Web(String),
}
fn resolve_link(path: &Path, target: &str) -> Option<Link> {
    if target.starts_with('#') {
        return None;
    }
    let base = Url::from_file_path(path).ok()?;
    let url = base.join(target).ok()?;
    match url.scheme() {
        "file" if path.starts_with("/__terminator_remote__") => None,
        "file" => url.to_file_path().ok().map(Link::File),
        "https" | "http" => Some(Link::Web(url.into())),
        _ => None,
    }
}
pub(super) struct Document {
    pub(super) text: String,
    pub(super) mermaid: Vec<super::mermaid::Diagram>,
    pub(super) words: usize,
    pub(super) headings: usize,
    pub(super) status: &'static str,
    pub(super) links: HashMap<String, Option<Link>>,
    pub(super) images: HashSet<String>,
}

/// Raw HTML has no native preview rendering: without a custom HTML hook the
/// viewer prints tags literally, so README-style headers (`p`, `h1`, badges
/// built from `a`/`img`, `br`) preview as tag soup. Convert that markup to
/// its readable text instead. Code spans never surface as HTML events, so
/// this cannot corrupt fenced or inline code.
fn html_to_markdown(chunk: &str) -> String {
    let mut out = String::with_capacity(chunk.len());
    let mut rest = chunk;
    while let Some(start) = rest.find('<') {
        out.push_str(rest.get(..start).unwrap_or(""));
        rest = rest.get(start..).unwrap_or("");
        if rest.starts_with("<!--") {
            rest = rest
                .find("-->")
                .and_then(|end| end.checked_add(3).and_then(|from| rest.get(from..)))
                .unwrap_or("");
            continue;
        }
        let Some(close) = rest.find('>') else {
            out.push_str(rest);
            break;
        };
        let tag = rest.get(1..close).unwrap_or("");
        rest = close
            .checked_add(1)
            .and_then(|from| rest.get(from..))
            .unwrap_or("");
        let (name, closing) = match tag.strip_prefix('/') {
            Some(name) => (name, true),
            None => (tag, false),
        };
        let name = name
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match name.as_str() {
            // Keep the break inside the Markdown line so table cells stay intact.
            // The Markdown parser decodes this entity to a literal text newline.
            "br" => out.push_str("&NewLine;"),
            "img" if !closing => {
                let alt = html_attr(tag, "alt").unwrap_or_default();
                if alt.is_empty() {
                    out.push_str("Image");
                } else {
                    out.push_str("Image: ");
                    out.push_str(&alt);
                }
            }
            "p" => out.push_str("\n\n"),
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" if !closing => {
                let level = name
                    .get(1..)
                    .and_then(|digits| digits.parse::<usize>().ok())
                    .unwrap_or(1);
                out.push_str("\n\n");
                out.push_str(&"#".repeat(level));
                out.push(' ');
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => out.push_str("\n\n"),
            _ => {}
        }
    }
    out.push_str(rest);
    collapse_html_text(&out)
}

fn html_attr(tag: &str, name: &str) -> Option<String> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len()
        && bytes
            .get(i)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b'/')
    {
        i = i.saturating_add(1);
    }
    loop {
        while i < bytes.len()
            && bytes
                .get(i)
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'/')
        {
            i = i.saturating_add(1);
        }
        if i >= bytes.len() {
            return None;
        }
        let start = i;
        while i < bytes.len()
            && bytes
                .get(i)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-' || *byte == b'_')
        {
            i = i.saturating_add(1);
        }
        if start == i {
            i = i.saturating_add(1);
            continue;
        }
        let attr = tag.get(start..i)?;
        let mut j = i;
        while j < bytes.len() && bytes.get(j).is_some_and(|byte| byte.is_ascii_whitespace()) {
            j = j.saturating_add(1);
        }
        if j < bytes.len() && bytes.get(j) == Some(&b'=') {
            j = j.saturating_add(1);
            while j < bytes.len() && bytes.get(j).is_some_and(|byte| byte.is_ascii_whitespace()) {
                j = j.saturating_add(1);
            }
            let value = if j < bytes.len()
                && bytes
                    .get(j)
                    .is_some_and(|byte| *byte == b'"' || *byte == b'\'')
            {
                let quote = bytes.get(j).copied()?;
                j = j.saturating_add(1);
                let start = j;
                while j < bytes.len() && bytes.get(j) != Some(&quote) {
                    j = j.saturating_add(1);
                }
                let value = tag.get(start..j)?.to_string();
                j = j.saturating_add(1).min(bytes.len());
                value
            } else {
                let start = j;
                while j < bytes.len()
                    && bytes.get(j).is_some_and(|byte| !byte.is_ascii_whitespace())
                {
                    j = j.saturating_add(1);
                }
                tag.get(start..j)?.to_string()
            };
            i = j;
            if attr.eq_ignore_ascii_case(name) {
                return Some(decode_html_entities(&value));
            }
        }
    }
}

fn decode_html_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(rest.get(..start).unwrap_or(""));
        let Some(tail) = rest.get(start..) else {
            break;
        };
        let candidate = tail
            .find(';')
            .filter(|&end| end < 12)
            .and_then(|end| tail.get(..=end))
            .filter(|entity| {
                entity
                    .get(1..)
                    .is_some_and(|body| !body.contains(|c: char| c.is_whitespace() || c == '<'))
            });
        if let Some((decoded, len)) = candidate.and_then(decode_entity) {
            out.push_str(&decoded);
            rest = tail.get(len..).unwrap_or("");
        } else {
            out.push('&');
            rest = tail.get(1..).unwrap_or("");
        }
    }
    out.push_str(rest);
    out
}

fn decode_entity(entity: &str) -> Option<(String, usize)> {
    let len = entity.len();
    let decoded = match entity {
        "&amp;" => "&".into(),
        "&lt;" => "<".into(),
        "&gt;" => ">".into(),
        "&quot;" => "\"".into(),
        "&#39;" | "&apos;" => "'".into(),
        "&nbsp;" => "\u{a0}".into(),
        _ => {
            let digits = entity.strip_prefix("&#")?.strip_suffix(';')?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            char::from_u32(digits.parse::<u32>().ok()?)
                .unwrap_or('\u{fffd}')
                .to_string()
        }
    };
    Some((decoded, len))
}

/// `as usize` for a UI width. Non-finite or negative becomes 0; overflow saturates.
pub(super) fn usize_from_f32(value: f32) -> usize {
    usize::try_from(u64_from_f32(value)).unwrap_or(usize::MAX)
}

fn u64_from_f32(value: f32) -> u64 {
    if value.is_nan() || value <= 0.0 {
        return 0;
    }
    if !value.is_finite() || value >= 18_446_744_073_709_551_616.0 {
        return u64::MAX;
    }
    let bits = value.to_bits();
    let Some(exp) = i32::try_from((bits >> 23) & 0xff)
        .ok()
        .and_then(|biased| biased.checked_sub(127))
    else {
        return 0;
    };
    if exp < 0 {
        return 0;
    }
    let mantissa = u64::from((bits & 0x007f_ffff) | (1_u32 << 23));
    let Some(shift) = exp.checked_sub(23) else {
        return 0;
    };
    if shift >= 0 {
        let Ok(places) = u32::try_from(shift) else {
            return u64::MAX;
        };
        mantissa.checked_shl(places).unwrap_or(u64::MAX)
    } else {
        mantissa.checked_shr(shift.unsigned_abs()).unwrap_or(0)
    }
}

fn collapse_html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines: u32 = 0;
    for ch in text.chars() {
        if ch == '\n' {
            newlines = newlines.saturating_add(1);
            if newlines <= 2 {
                out.push('\n');
            }
        } else {
            newlines = 0;
            out.push(ch);
        }
    }
    decode_html_entities(&out)
}
pub(super) fn prepare(snapshot: Snapshot) -> Document {
    let mut links = HashMap::new();
    let mut images = HashSet::new();
    let mut replacements = Vec::new();
    // Bare URLs stay plain text outside links and code blocks.
    let (mut in_link, mut in_code) = (0usize, false);
    // Merged text keeps a URL with `_` or `*` in one event.
    let mut events = TextMergeWithOffset::new(
        Parser::new_ext(&snapshot.text, Options::all()).into_offset_iter(),
    );
    while let Some((event, range)) = events.next() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                in_link = in_link.saturating_add(1);
                if !dest_url.starts_with('#') {
                    links.insert(
                        dest_url.to_string(),
                        resolve_link(&snapshot.path, &dest_url),
                    );
                }
            }
            Event::End(TagEnd::Link) => in_link = in_link.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => in_code = false,
            Event::Text(_) if in_link == 0 && !in_code => {
                let Some(source) = snapshot.text.get(range.clone()) else {
                    continue;
                };
                if let Some(linked) = autolink(source, &mut links) {
                    replacements.push((range, linked));
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let alt = image_alt(&mut events);
                let (replacement, uri) = image_replacement(&snapshot.path, &dest_url, &alt);
                images.extend(uri);
                replacements.push((range, replacement));
            }
            Event::Html(_) | Event::InlineHtml(_) => {
                let Some(html) = snapshot.text.get(range.clone()) else {
                    continue;
                };
                replacements.push((range, html_to_markdown(html)));
            }
            _ => {}
        }
    }
    let mut text = snapshot.text;
    for (range, replacement) in replacements.into_iter().rev() {
        text.replace_range(range, &replacement);
    }
    preserve_line_breaks(&mut text);
    let words = text.split_whitespace().count();
    let headings = count_headings(&text);
    let mermaid = super::mermaid::render_fences(&text);
    Document {
        text,
        mermaid,
        words,
        headings,
        status: snapshot_status(snapshot.revision, snapshot.paused),
        links,
        images,
    }
}

/// `source` with every bare http(s) URL wrapped as a CommonMark autolink,
/// or `None` when it has none.
fn autolink(source: &str, links: &mut HashMap<String, Option<Link>>) -> Option<String> {
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    let mut out = String::new();
    let mut position = 0;
    for link in finder.links(source) {
        let url = link.as_str();
        let Ok(parsed) = Url::parse(url) else {
            continue;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            continue;
        }
        out.push_str(source.get(position..link.start())?);
        out.push('<');
        out.push_str(url);
        out.push('>');
        position = link.end();
        links.insert(url.to_owned(), Some(Link::Web(parsed.into())));
    }
    if position == 0 {
        return None;
    }
    out.push_str(source.get(position..)?);
    Some(out)
}

/// The text of an image's alt, consuming events through its end tag.
fn image_alt<'a>(events: &mut impl Iterator<Item = (Event<'a>, std::ops::Range<usize>)>) -> String {
    let mut alt = String::new();
    for (event, _) in events.by_ref() {
        match event {
            Event::End(TagEnd::Image) => break,
            Event::Text(text) | Event::Code(text) => alt.push_str(&text),
            _ => {}
        }
    }
    alt.replace(['[', ']'], "")
}

/// Markdown for an image: a `markdown-image:` link (also returned, for the
/// loader) when it resolves to a local file, else a text placeholder.
fn image_replacement(base: &Path, dest_url: &str, alt: &str) -> (String, Option<String>) {
    let Some(Link::File(path)) = resolve_link(base, dest_url) else {
        return (format!("Image: {alt}"), None);
    };
    match Url::from_file_path(path) {
        Ok(url) => {
            let uri = format!("markdown-image:{url}");
            (format!("![{alt}](<{uri}>)"), Some(uri))
        }
        Err(()) => (format!("Image: {alt}"), None),
    }
}

/// Preserve source line breaks, including those in converted HTML blocks.
/// Parser offsets keep code, list/quote prefixes and existing hard breaks intact.
fn preserve_line_breaks(text: &mut String) {
    let breaks: Vec<_> = Parser::new_ext(text, Options::all())
        .into_offset_iter()
        .filter_map(|(event, range)| matches!(event, Event::SoftBreak).then_some(range))
        .collect();
    for range in breaks.into_iter().rev() {
        text.replace_range(range, "  \n");
    }
}

fn count_headings(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let hashes = line.bytes().take_while(|byte| *byte == b'#').count();
            (1..=6).contains(&hashes) && line.as_bytes().get(hashes) == Some(&b' ')
        })
        .count()
}

fn snapshot_status(revision: Option<Revision>, paused: bool) -> &'static str {
    match revision {
        Some(revision) if paused && revision.modified => "Unsaved preview · live updates paused",
        Some(_) if paused => "Last live preview · live updates paused",
        None if paused => "Saved file · live preview paused",
        Some(revision) if revision.modified => "Unsaved changes",
        Some(_) => "Live preview",
        None => "Saved file",
    }
}
