//! Prepare Mermaid fences on the Markdown CPU worker. Invalid diagrams stay
//! as source code in the ordinary Markdown viewer.
use eframe::egui;
use merman::svg::{RenderResourcePolicy, SvgPipeline};
use merman::{OperationControl, RenderOutput, RenderRequest, Renderer, SvgEnvironment, SvgRequest};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::VecDeque,
    ops::Range,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

const MAX_DIAGRAMS: usize = 8;
pub(super) const MAX_SOURCE_BYTES: usize = 32 * 1024;
const MAX_SVG_BYTES: usize = 2 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_CACHED_DIAGRAMS: usize = 16;
const RENDER_BUDGET: Duration = Duration::from_secs(2);

type DiagramCache = Mutex<VecDeque<(String, Arc<egui::ColorImage>)>>;
static CACHE: LazyLock<DiagramCache> = LazyLock::new(|| Mutex::new(VecDeque::new()));

pub(super) struct Diagram {
    pub range: Range<usize>,
    pub image: Arc<egui::ColorImage>,
}

/// Source cap of a standalone `.mmd`/`.mermaid` file (one diagram).
pub(super) const MAX_FILE_SOURCE_BYTES: usize = 256 * 1024;

/// Source cap per fence: larger for a standalone Mermaid file.
pub(super) const fn source_cap(diagram_file: bool) -> usize {
    if diagram_file {
        MAX_FILE_SOURCE_BYTES
    } else {
        MAX_SOURCE_BYTES
    }
}

/// `source` as one Mermaid fence, with a fence longer than any backtick
/// run inside it.
pub(super) fn fence_file(source: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for ch in source.chars() {
        run = if ch == '`' { run.saturating_add(1) } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest.saturating_add(1).max(3));
    let newline = if source.ends_with('\n') { "" } else { "\n" };
    format!("{fence}mermaid\n{source}{newline}{fence}\n")
}

/// True when `text` is exactly one rendered diagram and nothing else.
pub(super) fn single_diagram(text: &str, diagrams: &[Diagram]) -> bool {
    let [diagram] = diagrams else {
        return false;
    };
    text.get(..diagram.range.start)
        .is_some_and(|before| before.trim().is_empty())
        && text
            .get(diagram.range.end..)
            .is_some_and(|after| after.trim().is_empty())
}

#[cfg(test)]
pub(super) fn render_fences(text: &str) -> Vec<Diagram> {
    render_fences_limited(text, MAX_SOURCE_BYTES)
}

/// Rendered fences of `text`; a fence over `max_source` bytes stays source.
pub(super) fn render_fences_limited(text: &str, max_source: usize) -> Vec<Diagram> {
    let renderer = Renderer::new();
    let deadline = Instant::now()
        .checked_add(RENDER_BUDGET)
        .unwrap_or_else(Instant::now);
    let mut diagrams = Vec::new();
    let mut current: Option<(usize, String)> = None;
    let mut container_depth = 0usize;
    let mut used_bytes = 0usize;
    for (event, range) in Parser::new_ext(text, Options::all()).into_offset_iter() {
        match event {
            Event::Start(
                Tag::BlockQuote(_) | Tag::List(_) | Tag::Item | Tag::FootnoteDefinition(_),
            ) => {
                container_depth = container_depth.saturating_add(1);
            }
            Event::End(
                TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition,
            ) => {
                container_depth = container_depth.saturating_sub(1);
            }
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                if container_depth == 0
                    && info
                        .split_whitespace()
                        .next()
                        .is_some_and(|language| language.eq_ignore_ascii_case("mermaid")) =>
            {
                current = Some((range.start, String::new()));
            }
            Event::Text(content) if current.is_some() => {
                if let Some((_, source)) = current.as_mut()
                    && source.len().saturating_add(content.len()) <= max_source
                {
                    source.push_str(&content);
                } else {
                    current = None;
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, source)) = current.take()
                    && diagrams.len() < MAX_DIAGRAMS
                    && !source.trim().is_empty()
                    && let Some(image) = cached_render(&renderer, &source, deadline)
                {
                    let bytes = image.pixels.len().saturating_mul(4);
                    if used_bytes.saturating_add(bytes) <= MAX_IMAGE_BYTES {
                        used_bytes = used_bytes.saturating_add(bytes);
                        diagrams.push(Diagram {
                            range: start..range.end,
                            image,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    diagrams
}

fn cached_render(
    renderer: &Renderer,
    source: &str,
    deadline: Instant,
) -> Option<Arc<egui::ColorImage>> {
    {
        let mut cache = CACHE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(index) = cache.iter().position(|(cached, _)| cached == source)
            && let Some(entry) = cache.remove(index)
        {
            let image = Arc::clone(&entry.1);
            cache.push_back(entry);
            return Some(image);
        }
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return None;
    }
    let image = Arc::new(render(renderer, source, remaining)?);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.push_back((source.to_owned(), Arc::clone(&image)));
    while cache.len() > MAX_CACHED_DIAGRAMS
        || cache
            .iter()
            .map(|(_, image)| image.pixels.len().saturating_mul(4))
            .sum::<usize>()
            > MAX_IMAGE_BYTES
    {
        cache.pop_front();
    }
    Some(image)
}

fn render(renderer: &Renderer, source: &str, remaining: Duration) -> Option<egui::ColorImage> {
    let request = SvgRequest {
        environment: SvgEnvironment::deterministic()
            .with_resource_policy(RenderResourcePolicy::interactive()),
        pipeline: Some(SvgPipeline::resvg_safe()),
        ..SvgRequest::default()
    };
    let output = renderer
        .render(RenderRequest::svg(
            source,
            OperationControl::new().with_deadline(remaining),
            request,
        ))
        .ok()?;
    let RenderOutput::Svg(Some(svg)) = output else {
        return None;
    };
    let bytes = svg.svg().as_bytes();
    if bytes.len() > MAX_SVG_BYTES {
        return None;
    }
    crate::image_preview::decode_bytes(bytes.to_vec(), true).ok()
}

#[cfg(test)]
mod tests {
    use super::render_fences;

    #[test]
    fn mermaid_fence_renders_without_consuming_surrounding_markdown() {
        let markdown = "# Before\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n[After](next.md)\n";
        let diagrams = render_fences(markdown);
        assert_eq!(diagrams.len(), 1);
        assert!(
            markdown
                .get(diagrams[0].range.clone())
                .is_some_and(|text| text.starts_with("```mermaid"))
        );
        assert!(
            markdown
                .get(..diagrams[0].range.start)
                .is_some_and(|text| text.contains("# Before"))
        );
        assert!(
            markdown
                .get(diagrams[0].range.end..)
                .is_some_and(|text| text.contains("[After](next.md)"))
        );
        assert!(!diagrams[0].image.pixels.is_empty());
    }

    #[test]
    fn invalid_mermaid_and_ordinary_code_remain_source() {
        let markdown = "```mermaid\nnot a Mermaid diagram\n```\n\n```rust\nfn main() {}\n```";
        assert!(render_fences(markdown).is_empty());
    }

    #[test]
    fn mermaid_files_get_the_markdown_preview_and_other_files_do_not() {
        use super::super::source::{mermaid_file, supported};
        use std::path::Path;
        for name in ["flow.mmd", "flow.MMD", "flow.mermaid", "notes.md"] {
            assert!(supported(Path::new(name)), "{name}");
        }
        assert!(mermaid_file(Path::new("a.Mermaid")));
        assert!(!mermaid_file(Path::new("a.md")));
        assert!(!supported(Path::new("data.csv")));
    }

    #[test]
    fn mermaid_file_renders_as_one_zoomable_diagram_with_a_larger_cap() {
        let mut source = String::from("flowchart LR\n  A --> B\n");
        // Over the inline cap, under the file cap.
        while source.len() <= super::MAX_SOURCE_BYTES {
            source.push_str("  %% a comment line that pads the diagram source\n");
        }
        let text = super::fence_file(&source);
        assert!(render_fences(&text).is_empty());
        let diagrams = super::render_fences_limited(&text, super::MAX_FILE_SOURCE_BYTES);
        assert_eq!(diagrams.len(), 1);
        assert!(super::single_diagram(&text, &diagrams));
        let mixed = "# Title\n\n```mermaid\nflowchart LR\n A --> B\n```\n";
        let diagrams = render_fences(mixed);
        assert_eq!(diagrams.len(), 1);
        assert!(!super::single_diagram(mixed, &diagrams));
    }

    #[test]
    fn mermaid_file_fence_outlasts_backticks_in_the_source() {
        let fenced = super::fence_file("flowchart LR\n A[\"````\"] --> B");
        assert!(fenced.starts_with("`````mermaid\n"));
        assert!(fenced.ends_with("\n`````\n"));
    }

    #[test]
    fn nested_mermaid_keeps_its_markdown_container() {
        let markdown = "> ```mermaid\n> flowchart LR\n> A --> B\n> ```\n";
        assert!(render_fences(markdown).is_empty());
    }
}
