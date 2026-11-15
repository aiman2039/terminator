use std::iter::Peekable;
use std::ops::Range;

use crate::{CommonMarkCache, CommonMarkOptions};

use egui::{self, Id, TextStyle, Ui};

use crate::List;
use egui_commonmark_backend::elements::*;
use egui_commonmark_backend::misc::*;
use egui_commonmark_backend::pulldown::*;
use pulldown_cmark::{CowStr, HeadingLevel};

/// Backend 0.26 stores scroll state on [`ViewerCache`], keyed by source id.
fn scroll_cache<'a>(cache: &'a mut CommonMarkCache, id: &Id) -> &'a mut ViewerCache {
    viewer_cache(cache, id)
}

fn viewer_id(options: &CommonMarkOptions) -> Id {
    options.source_id.unwrap_or(Id::NULL)
}

/// Newline logic is constructed by the following:
/// All elements try to insert a newline before them (if they are allowed)
/// and end their own line.
struct Newline {
    /// Whether a newline should not be inserted before a widget. This is only for
    /// the first widget.
    should_not_start_newline_forced: bool,
    /// Whether an element should insert a newline before it
    should_start_newline: bool,
    /// Whether an element should end it's own line using a newline
    /// This will have to be set to false in cases such as when blocks are within
    /// a list.
    should_end_newline: bool,
    /// only false when the widget is the last one.
    should_end_newline_forced: bool,
}

impl Default for Newline {
    fn default() -> Self {
        Self {
            should_not_start_newline_forced: true,
            should_start_newline: true,
            should_end_newline: true,
            should_end_newline_forced: true,
        }
    }
}

impl Newline {
    pub fn can_insert_end(&self) -> bool {
        self.should_end_newline && self.should_end_newline_forced
    }

    pub fn can_insert_start(&self) -> bool {
        self.should_start_newline && !self.should_not_start_newline_forced
    }

    pub fn try_insert_start(&self, ui: &mut Ui) {
        if self.can_insert_start() {
            newline(ui);
        }
    }

    pub fn try_insert_end(&self, ui: &mut Ui) {
        if self.can_insert_end() {
            newline(ui);
        }
    }
}

#[derive(Default)]
struct DefinitionList {
    is_first_item: bool,
    is_def_list_def: bool,
}

pub struct CommonMarkViewerInternal {
    curr_table: usize,
    text_style: Style,
    list: List,
    link: Option<Link>,
    image: Option<Image>,
    line: Newline,
    code_block: Option<CodeBlock>,

    /// Only populated if the html_fn option has been set
    html_block: String,
    is_list_item: bool,
    def_list: DefinitionList,
    is_table: bool,
    is_blockquote: bool,
    checkbox_events: Vec<CheckboxClickEvent>,
    deferred_scroll_to_heading: Option<String>,
    /// Painted area of the current heading's text, for the H1/H2 divider.
    heading_rect: Option<egui::Rect>,
    /// Styled text runs not painted yet. Adjacent runs (plain, bold,
    /// italic, code) paint as one label, so text wraps only at spaces:
    /// separate labels each wrapped on their own, which put the full stop
    /// after `**bold**` text alone on the next row.
    inline: Option<egui::text::LayoutJob>,
    /// Source starts of the list items that are task items (`- [x]`).
    task_items: std::collections::HashSet<usize>,
    /// The item being started is a task item.
    item_is_task: bool,
    /// Text is in a table header cell.
    table_header: bool,
}

/// Source starts of the list items in `text` that begin with a task list
/// marker. A marker belongs to the innermost open item.
fn task_item_starts(
    text: &str,
    options: pulldown_cmark::Options,
) -> std::collections::HashSet<usize> {
    let mut starts = std::collections::HashSet::new();
    if !options.contains(pulldown_cmark::Options::ENABLE_TASKLISTS) || !text.contains('[') {
        return starts;
    }
    let mut open = Vec::new();
    for (event, range) in pulldown_cmark::Parser::new_ext(text, options).into_offset_iter() {
        match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Item) => open.push(range.start),
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Item) => {
                open.pop();
            }
            pulldown_cmark::Event::TaskListMarker(_) => {
                if let Some(start) = open.last() {
                    starts.insert(*start);
                }
            }
            _ => {}
        }
    }
    starts
}

pub(crate) struct CheckboxClickEvent {
    pub(crate) checked: bool,
    pub(crate) span: Range<usize>,
}

impl CommonMarkViewerInternal {
    pub fn new() -> Self {
        Self {
            curr_table: 0,
            text_style: Style::default(),
            list: List::default(),
            link: None,
            image: None,
            line: Newline::default(),
            is_list_item: false,
            def_list: Default::default(),
            code_block: None,
            html_block: String::new(),
            is_table: false,
            is_blockquote: false,
            checkbox_events: Vec::new(),
            deferred_scroll_to_heading: None,
            heading_rect: None,
            inline: None,
            task_items: std::collections::HashSet::new(),
            item_is_task: false,
            table_header: false,
        }
    }

    /// Paint the pending text runs as one label.
    fn flush_inline(&mut self, ui: &mut Ui) {
        if let Some(job) = self.inline.take() {
            let rect = ui.label(job).rect;
            if self.text_style.heading.is_some() {
                self.heading_rect = Some(self.heading_rect.map_or(rect, |r| r.union(rect)));
            }
        }
    }

    /// Add a text run to the pending label.
    fn push_inline(&mut self, ui: &Ui, text: egui::RichText) {
        let job = self.inline.get_or_insert_with(Default::default);
        text.append_to(
            job,
            ui.style(),
            egui::FontSelection::Default,
            ui.text_valign(),
        );
    }
}

type TableCell<'e> = Vec<(pulldown_cmark::Event<'e>, Range<usize>)>;

/// Space between two table columns. The viewer sets no item spacing
/// between labels, so the gap is added explicitly.
fn table_column_gap(ui: &Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Body) * 0.75).round()
}

/// The face for bold text (`**strong**`, table headers): the `Heading`
/// family, a bold face when the app sets one.
fn bold_family(ui: &Ui) -> Option<egui::FontFamily> {
    ui.style()
        .text_styles
        .get(&TextStyle::Heading)
        .map(|font| font.family.clone())
}

/// The text runs of a table cell as `(text, code, bold)`.
fn cell_runs<'a>(
    cell: &'a TableCell<'_>,
    is_header: bool,
) -> impl Iterator<Item = (&'a str, bool, bool)> {
    let mut bold = is_header;
    cell.iter().filter_map(move |(event, _)| match event {
        pulldown_cmark::Event::Start(pulldown_cmark::Tag::Strong) => {
            bold = true;
            None
        }
        pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Strong) => {
            bold = is_header;
            None
        }
        pulldown_cmark::Event::Text(text) => Some((text.as_ref(), false, bold)),
        pulldown_cmark::Event::Code(text) => Some((text.as_ref(), true, bold)),
        _ => None,
    })
}

/// Whole-point column widths that fit a table into `available`, and whether
/// they do, close to the CSS automatic table layout: when the unwrapped
/// cells do not fit, each column gets its longest word, and the rest of the
/// width goes to the columns in proportion to how much they would need to
/// stay unwrapped. When even the longest words do not fit, the columns keep
/// them and the table scrolls.
fn table_column_widths(
    ui: &Ui,
    header: &[TableCell<'_>],
    rows: &[Vec<TableCell<'_>>],
    available: f32,
) -> (Vec<f32>, bool) {
    let columns = header.len();
    let bold_family = bold_family(ui);
    // A little room for rounding to whole points.
    let measure = |text: &str, code: bool, bold: bool, is_header: bool| {
        let mut text = egui::RichText::new(text);
        if code {
            text = text.code();
        } else if bold && let Some(family) = &bold_family {
            text = text.family(family.clone());
        }
        // Header text is small (see `event_text`).
        let style = if is_header && !code {
            TextStyle::Small
        } else {
            TextStyle::Body
        };
        egui::WidgetText::from(text)
            .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style)
            .size()
            .x
            + 2.0
    };
    let all_rows = || {
        let body = rows.iter().map(|row| (false, row.as_slice()));
        std::iter::once((true, header)).chain(body)
    };
    let available = available - table_column_gap(ui) * columns.saturating_sub(1) as f32;
    let whole = |widths: Vec<f32>| widths.into_iter().map(f32::ceil).collect::<Vec<_>>();

    let mut natural = vec![0.0_f32; columns];
    for (is_header, row) in all_rows() {
        for (index, cell) in row.iter().enumerate().take(columns) {
            let width = cell_runs(cell, is_header)
                .map(|(text, code, bold)| measure(text, code, bold, is_header))
                .sum::<f32>();
            natural[index] = natural[index].max(width);
        }
    }
    let natural = whole(natural);
    let natural_sum: f32 = natural.iter().sum();
    if natural_sum <= available {
        // The table takes the content width (CSS `width: 100%`): the spare
        // width goes to the columns in proportion to their unwrapped width.
        if natural_sum <= 0.0 {
            return (natural, true);
        }
        let scale = available / natural_sum;
        let widths = natural
            .iter()
            .map(|width| (width * scale).floor())
            .collect();
        return (widths, true);
    }

    // Only a table that overflows measures its words.
    let mut minimum = vec![0.0_f32; columns];
    for (is_header, row) in all_rows() {
        for (index, cell) in row.iter().enumerate().take(columns) {
            for (text, code, bold) in cell_runs(cell, is_header) {
                for word in text.split_whitespace() {
                    minimum[index] = minimum[index].max(measure(word, code, bold, is_header));
                }
            }
        }
    }
    let minimum: Vec<f32> = whole(minimum)
        .into_iter()
        .zip(&natural)
        .map(|(word, natural)| word.min(*natural))
        .collect();
    let minimum_sum: f32 = minimum.iter().sum();
    if minimum_sum >= available {
        return (minimum, false);
    }
    let extra = available - minimum_sum;
    let flexible = natural_sum - minimum_sum;
    let widths = natural
        .iter()
        .zip(&minimum)
        .map(|(natural, minimum)| (minimum + extra * (natural - minimum) / flexible).floor())
        .collect();
    (widths, true)
}

/// Heading text in the `Heading` text style family (a bold face when the
/// app sets one) at clearly stepped sizes between `Body` and `Heading`.
/// The backend only colors levels 2-6 strong, at sizes close to H1.
fn heading_text(ui: &Ui, level: u8, text: egui::RichText) -> egui::RichText {
    let styles = &ui.style().text_styles;
    let Some(heading) = styles.get(&TextStyle::Heading) else {
        return text;
    };
    let body = styles.get(&TextStyle::Body).map_or(14.0, |font| font.size);
    let step = [1.0, 0.55, 0.3, 0.12, 0.0, 0.0][usize::from(level.min(5))];
    text.family(heading.family.clone())
        .size(body + (heading.size - body) * step)
}

fn parser_options_extras(
    is_math_enabled: bool,
    is_scroll_to_heading_enabled: bool,
) -> pulldown_cmark::Options {
    let mut result = parser_options();
    if is_math_enabled {
        result |= pulldown_cmark::Options::ENABLE_MATH;
    }
    if is_scroll_to_heading_enabled {
        result |= pulldown_cmark::Options::ENABLE_HEADING_ATTRIBUTES;
    }
    result
}

// Markdown discards repeated blank lines between blocks. Keep their extra
// display rows without changing the parsed blocks or source offsets.
fn spaced_events(
    text: &str,
    options: pulldown_cmark::Options,
) -> impl Iterator<Item = (pulldown_cmark::Event<'_>, Range<usize>)> {
    let mut previous_end = 0;
    pulldown_cmark::Parser::new_ext(text, options)
        .into_offset_iter()
        .flat_map(move |(event, range)| {
            let mut extra = 0;
            if matches!(event, pulldown_cmark::Event::Start(_)) && range.start > previous_end {
                let mut gap = &text[previous_end..range.start];
                if previous_end > 0 && !text[..previous_end].ends_with('\n') {
                    gap = gap
                        .strip_prefix("\r\n")
                        .or_else(|| gap.strip_prefix('\n'))
                        .unwrap_or(gap);
                }
                if gap.chars().all(|ch| ch.is_whitespace() || ch == '>') {
                    let blank_lines = gap
                        .split_inclusive('\n')
                        .filter(|line| line.ends_with('\n'))
                        .count();
                    // Block boundaries already provide one blank line.
                    extra = blank_lines.saturating_sub(usize::from(previous_end > 0));
                    previous_end = range.start;
                }
            }
            if matches!(
                event,
                pulldown_cmark::Event::Text(_)
                    | pulldown_cmark::Event::Code(_)
                    | pulldown_cmark::Event::HardBreak
                    | pulldown_cmark::Event::SoftBreak
                    | pulldown_cmark::Event::Rule
                    | pulldown_cmark::Event::End(
                        pulldown_cmark::TagEnd::Paragraph
                            | pulldown_cmark::TagEnd::Heading(_)
                            | pulldown_cmark::TagEnd::CodeBlock
                            | pulldown_cmark::TagEnd::Table
                    )
            ) {
                previous_end = previous_end.max(range.end);
            }
            std::iter::repeat_n(
                (pulldown_cmark::Event::HardBreak, range.start..range.start),
                extra,
            )
            .chain(std::iter::once((event, range)))
        })
}

impl CommonMarkViewerInternal {
    /// Be aware that this acquires egui::Context internally.
    /// If split Id is provided then split points will be populated
    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        text: &str,
        split_points_id: Option<Id>,
    ) -> (egui::InnerResponse<()>, Vec<CheckboxClickEvent>) {
        let max_width = options.max_width(ui);
        let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);

        let re = ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let height = ui.text_style_height(&TextStyle::Body);
            ui.set_row_height(height);

            let parser_options =
                parser_options_extras(options.math_fn.is_some(), options.enable_scroll_to_heading);
            self.task_items = task_item_starts(text, parser_options);
            let mut events = spaced_events(text, parser_options).enumerate().peekable();

            while let Some((index, (e, src_span))) = events.next() {
                let start_position = ui.next_widget_position();
                let is_element_end = matches!(e, pulldown_cmark::Event::End(_));
                let should_add_split_point = self.list.is_inside_a_list() && is_element_end;

                if events.peek().is_none() {
                    self.line.should_end_newline_forced = false;
                }

                self.process_event(ui, &mut events, e, src_span, cache, options, max_width);

                if let Some(source_id) = split_points_id
                    && should_add_split_point
                {
                    let scroll_cache = scroll_cache(cache, &source_id);
                    let end_position = ui.next_widget_position();

                    let split_point_exists = scroll_cache
                        .split_points
                        .iter()
                        .any(|point| point.event_index == index);

                    if !split_point_exists {
                        scroll_cache.split_points.push(SplitPoint {
                            event_index: index,
                            vstart: start_position,
                            vend: end_position,
                            src_span: 0..0,
                        });
                    }
                }

                if index == 0 {
                    self.line.should_not_start_newline_forced = false;
                }
            }
            self.flush_inline(ui);

            // deferral to make it consistent no matter whether the target is before or after the link
            let source_id = viewer_id(options);
            // Terminator: keep a target set earlier until its heading takes
            // it; a later show call (another block) must not clear it.
            if let Some(target) = self.deferred_scroll_to_heading.take() {
                *scroll_cache(cache, &source_id).scroll_to_id_target_mut() = Some(target);
            }

            if let Some(source_id) = split_points_id {
                scroll_cache(cache, &source_id).page_size =
                    Some(ui.next_widget_position().to_vec2());
            }
        });

        (re, std::mem::take(&mut self.checkbox_events))
    }

    #[cfg(feature = "pulldown_cmark")]
    pub(crate) fn show_scrollable(
        &mut self,
        source_id: Id,
        ui: &mut egui::Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        text: &str,
    ) {
        let available_size = ui.available_size();
        let scroll_id = source_id.with("_scroll_area");

        let Some(page_size) = scroll_cache(cache, &source_id).page_size else {
            egui::ScrollArea::vertical()
                .id_salt(scroll_id)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    self.show(ui, cache, options, text, Some(source_id));
                });
            // Prevent repopulating points twice at startup
            scroll_cache(cache, &source_id).available_size = available_size;
            return;
        };

        let parser_options =
            parser_options_extras(options.math_fn.is_some(), options.enable_scroll_to_heading);
        self.task_items = task_item_starts(text, parser_options);
        let events = spaced_events(text, parser_options).collect::<Vec<_>>();

        let num_rows = events.len();

        egui::ScrollArea::vertical()
            .id_salt(scroll_id)
            // Elements have different widths, so the scroll area cannot try to shrink to the
            // content, as that will mean that the scroll bar will move when loading elements
            // with different widths.
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                ui.set_height(page_size.y);
                let layout = egui::Layout::left_to_right(egui::Align::BOTTOM).with_main_wrap(true);

                let max_width = options.max_width(ui);
                ui.allocate_ui_with_layout(egui::vec2(max_width, 0.0), layout, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let scroll_cache = scroll_cache(cache, &source_id);

                    // finding the first element that's not in the viewport anymore
                    let (first_event_index, _, first_end_position) = scroll_cache
                        .split_points
                        .iter()
                        .filter(|point| point.vend.y < viewport.min.y)
                        .nth_back(1)
                        .map(|point| (point.event_index, point.vstart, point.vend))
                        .unwrap_or((0, egui::Pos2::ZERO, egui::Pos2::ZERO));

                    // finding the last element that's just outside the viewport
                    let last_event_index = scroll_cache
                        .split_points
                        .iter()
                        .filter(|point| point.vstart.y > viewport.max.y)
                        .nth(1)
                        .map(|point| point.event_index)
                        .unwrap_or(num_rows);

                    ui.allocate_space(first_end_position.to_vec2());

                    // only rendering the elements that are inside the viewport
                    let mut events = events
                        .into_iter()
                        .enumerate()
                        .skip(first_event_index)
                        .take(last_event_index - first_event_index)
                        .peekable();

                    while let Some((i, (e, src_span))) = events.next() {
                        if events.peek().is_none() {
                            self.line.should_end_newline_forced = false;
                        }

                        self.process_event(ui, &mut events, e, src_span, cache, options, max_width);

                        if i == 0 {
                            self.line.should_not_start_newline_forced = false;
                        }
                    }
                    self.flush_inline(ui);
                });
            });

        // Forcing full re-render to repopulate split points for the new size
        let scroll_cache = scroll_cache(cache, &source_id);
        if available_size != scroll_cache.available_size {
            scroll_cache.available_size = available_size;
            scroll_cache.page_size = None;
            scroll_cache.split_points.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process_event<'e>(
        &mut self,
        ui: &mut Ui,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        let ends_list = matches!(
            event,
            pulldown_cmark::Event::End(pulldown_cmark::TagEnd::List(_))
        );
        self.event(ui, event, src_span, cache, options, max_width);

        // Item content after a nested list (such as a second paragraph)
        // starts a new row at the item's text indent and wraps there, like
        // the content before the nested list.
        if ends_list
            && self.list.is_inside_a_list()
            && !matches!(
                events.peek(),
                None | Some((
                    _,
                    (pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Item), _)
                ))
            )
        {
            self.list.continue_item(ui, options);
            self.is_list_item = true;
        }

        self.def_list_def_wrapping(events, max_width, cache, options, ui);
        self.item_list_wrapping(events, max_width, cache, options, ui);
        self.table(events, cache, options, ui, max_width);
        self.blockquote(events, max_width, cache, options, ui);
    }

    fn def_list_def_wrapping<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.def_list.is_def_list_def {
            self.def_list.is_def_list_def = false;

            let item_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::DefinitionListDefinition)
            });

            let mut events_iter = item_events.into_iter().enumerate().peekable();

            self.line.try_insert_start(ui);

            // Proccess a single event separately so that we do not insert spaces where we do not
            // want them
            self.line.should_start_newline = false;
            if let Some((_, (e, src_span))) = events_iter.next() {
                self.process_event(ui, &mut events_iter, e, src_span, cache, options, max_width);
            }
            self.flush_inline(ui);

            ui.label(" ".repeat(options.indentation_spaces));
            self.line.should_start_newline = true;
            self.line.should_end_newline = false;
            // Required to ensure that the content is aligned with the identation
            ui.horizontal_wrapped(|ui| {
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
                self.flush_inline(ui);
            });
            self.line.should_end_newline = true;

            // Only end the definition items line if it is not the last element in the list
            if !matches!(
                events.peek(),
                Some((
                    _,
                    (
                        pulldown_cmark::Event::End(pulldown_cmark::TagEnd::DefinitionList),
                        _
                    )
                ))
            ) {
                self.line.try_insert_end(ui);
            }
        }
    }

    fn item_list_wrapping<'e>(
        &mut self,
        events: &mut impl Iterator<Item = EventIteratorItem<'e>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_list_item {
            self.is_list_item = false;

            let item_events = delayed_events_list_item(events);
            let mut events_iter = item_events.into_iter().enumerate().peekable();

            // Required to ensure that the content of the list item is aligned with
            // the * or - when wrapping
            ui.horizontal_wrapped(|ui| {
                // List text must use the same row height as its marker.
                ui.set_row_height(ui.text_style_height(&TextStyle::Body));
                while let Some((_, (e, src_span))) = events_iter.next() {
                    self.process_event(
                        ui,
                        &mut events_iter,
                        e,
                        src_span,
                        cache,
                        options,
                        max_width,
                    );
                }
                self.flush_inline(ui);
            });
        }
    }

    fn blockquote<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        max_width: f32,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
    ) {
        if self.is_blockquote {
            let mut collected_events = delayed_events(events, |tag| {
                matches!(tag, pulldown_cmark::TagEnd::BlockQuote(_))
            });
            self.line.try_insert_start(ui);

            // Currently the blockquotes are made in such a way that they need a newline at the end
            // and the start so when this is the first element in the markdown the newline must be
            // manually enabled
            self.line.should_not_start_newline_forced = false;
            if let Some(alert) = parse_alerts(&options.alerts, &mut collected_events) {
                egui_commonmark_backend::alert_ui(alert, ui, |ui| {
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                    self.flush_inline(ui);
                })
            } else {
                blockquote(ui, ui.visuals().weak_text_color(), |ui| {
                    self.text_style.quote = true;
                    for (event, src_span) in collected_events {
                        self.event(ui, event, src_span, cache, options, max_width);
                    }
                    self.flush_inline(ui);
                    self.text_style.quote = false;
                });
            }

            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
            self.is_blockquote = false;
        }
    }

    fn table<'e>(
        &mut self,
        events: &mut Peekable<impl Iterator<Item = EventIteratorItem<'e>>>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        ui: &mut Ui,
        max_width: f32,
    ) {
        if self.is_table {
            self.line.try_insert_start(ui);

            let id = ui.scope_id().with("_table").with(self.curr_table);
            self.curr_table += 1;

            let Table { header, rows } = parse_table(events);
            // Measured outside the scroll area: inside it the width is unbounded.
            let available = ui.available_width().min(max_width);
            let (widths, fits) = table_column_widths(ui, &header, &rows, available);

            let draw = |ui: &mut Ui| {
                // The document flows in a wrapping row layout; rows of a
                // table stack top to bottom. No outer box and no stripes:
                // a thin rule under the header and between rows.
                ui.vertical(|ui| {
                    let gap = ui.spacing().item_spacing.y;
                    let rule = ui.visuals().widgets.noninteractive.bg_stroke;
                    let header = std::iter::once((true, header));
                    // `parse_table` ends with an empty row.
                    let body = rows
                        .into_iter()
                        .filter(|row| !row.is_empty())
                        .map(|row| (false, row));
                    let all: Vec<_> = header.chain(body).collect();
                    let last = all.len().saturating_sub(1);
                    for (row_index, (is_header, row)) in all.into_iter().enumerate() {
                        let row_rect =
                            self.table_row(ui, row, &widths, is_header, cache, options, max_width);
                        if row_index < last {
                            let y = row_rect.bottom() + 0.5 * gap;
                            ui.painter().hline(row_rect.x_range(), y, rule);
                        }
                    }
                });
            };
            // Long cells wrap so the table fits the text column, as in a
            // browser. A table that still does not fit (long unbreakable
            // words) scrolls sideways instead of being clipped. Only then:
            // the scroll area keeps room for its bar below the table.
            if fits {
                draw(ui);
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt(id.with("_scroll"))
                    .auto_shrink([false, true])
                    .show(ui, draw);
            }

            self.is_table = false;
            if events.peek().is_none() {
                self.line.should_end_newline_forced = false;
            }

            self.line.try_insert_end(ui);
        }
    }

    /// One table row, laid out directly rather than with `egui::Grid`: a
    /// Grid places cells by the previous frame's row heights, so a wrapped
    /// cell overlapped the next row. Here the row is as tall as its tallest
    /// cell. Returns the row's rectangle.
    #[allow(clippy::too_many_arguments)]
    fn table_row(
        &mut self,
        ui: &mut Ui,
        row: Vec<TableCell<'_>>,
        widths: &[f32],
        is_header: bool,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) -> egui::Rect {
        let column_gap = table_column_gap(ui);
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(0, 4))
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (index, cell) in row.into_iter().enumerate() {
                        if index > 0 {
                            ui.add_space(column_gap);
                        }
                        let width = widths.get(index).copied().unwrap_or(max_width);
                        self.table_cell(ui, cell, width, is_header, cache, options, max_width);
                    }
                });
            })
            .response
            .rect
    }

    /// One table cell: exactly `width` wide (so the columns line up), its
    /// content wrapping inside. Header cells are bold, small and muted.
    #[allow(clippy::too_many_arguments)]
    fn table_cell(
        &mut self,
        ui: &mut Ui,
        cell: TableCell<'_>,
        width: f32,
        is_header: bool,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        let row_height = ui.text_style_height(&TextStyle::Body);
        let layout = egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true);
        ui.allocate_ui_with_layout(egui::vec2(width, row_height), layout, |ui| {
            ui.set_max_width(width);
            ui.set_row_height(row_height);
            let strong = self.text_style.strong;
            self.text_style.strong |= is_header;
            self.table_header = is_header;
            for (e, src_span) in cell {
                let tmp_start = std::mem::replace(&mut self.line.should_start_newline, false);
                let tmp_end = std::mem::replace(&mut self.line.should_end_newline, false);
                self.event(ui, e, src_span, cache, options, max_width);
                self.line.should_start_newline = tmp_start;
                self.line.should_end_newline = tmp_end;
            }
            self.flush_inline(ui);
            self.table_header = false;
            self.text_style.strong = strong;
            // The full column width, after the content: a minimum width set
            // first counts as an earlier item (the first label then starts
            // one item spacing in), and `set_min_width` here would measure
            // from the cursor instead of the cell's left edge.
            ui.expand_to_include_x(ui.max_rect().left() + width);
        });
    }

    fn event(
        &mut self,
        ui: &mut Ui,
        event: pulldown_cmark::Event,
        src_span: Range<usize>,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        use pulldown_cmark::{Event, Tag, TagEnd};
        // Runs of styled text collect into one label; anything else paints
        // the collected text first.
        let in_text_widget = self.link.is_some() || self.image.is_some();
        let inline = match &event {
            Event::Text(_) | Event::Code(_) | Event::InlineHtml(_) => true,
            Event::SoftBreak => !in_text_widget && self.code_block.is_none(),
            Event::Start(Tag::Emphasis | Tag::Strong | Tag::Strikethrough)
            | Event::End(TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough) => true,
            _ => false,
        };
        if !inline {
            self.flush_inline(ui);
        }
        if matches!(event, Event::Start(Tag::Item)) {
            self.item_is_task = self.task_items.contains(&src_span.start);
        }
        match event {
            pulldown_cmark::Event::Start(tag) => self.start_tag(ui, tag, cache, options),
            pulldown_cmark::Event::End(tag) => self.end_tag(ui, tag, cache, options, max_width),
            pulldown_cmark::Event::Text(text) => {
                self.event_text(text, ui);
            }
            pulldown_cmark::Event::Code(text) => {
                self.text_style.code = true;
                self.event_text(text, ui);
                self.text_style.code = false;
            }
            pulldown_cmark::Event::InlineHtml(text) => {
                self.event_text(text, ui);
            }

            pulldown_cmark::Event::Html(text) => {
                if options.html_fn.is_some() {
                    self.html_block.push_str(&text);
                } else {
                    self.event_text(text, ui);
                }
            }
            pulldown_cmark::Event::FootnoteReference(footnote) => {
                footnote_start(ui, &footnote);
            }
            pulldown_cmark::Event::SoftBreak if inline => {
                self.push_inline(ui, egui::RichText::new(" "));
            }
            pulldown_cmark::Event::SoftBreak => {
                soft_break(ui);
            }
            pulldown_cmark::Event::HardBreak => newline(ui),
            pulldown_cmark::Event::Rule => {
                self.line.try_insert_start(ui);
                rule(ui, self.line.can_insert_end());
            }
            pulldown_cmark::Event::TaskListMarker(mut checkbox) => {
                if options.mutable {
                    if ui
                        .add(egui::Checkbox::without_text(&mut checkbox))
                        .clicked()
                    {
                        self.checkbox_events.push(CheckboxClickEvent {
                            checked: checkbox,
                            span: src_span,
                        });
                    }
                } else {
                    ui.add(ImmutableCheckbox::without_text(&mut checkbox));
                }
            }
            pulldown_cmark::Event::InlineMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, true);
                }
            }
            pulldown_cmark::Event::DisplayMath(tex) => {
                if let Some(math_fn) = options.math_fn {
                    math_fn(ui, &tex, false);
                }
            }
        }
    }

    fn event_text(&mut self, text: CowStr, ui: &mut Ui) {
        let mut rich_text = self.text_style.to_richtext(ui, &text);
        if let Some(level) = self.text_style.heading
            && !self.text_style.code
        {
            rich_text = heading_text(ui, level, rich_text);
        } else if self.text_style.strong
            && !self.text_style.code
            && let Some(family) = bold_family(ui)
        {
            // The backend only gives strong text the strong color.
            rich_text = rich_text.family(family);
        }
        if self.table_header && !self.text_style.code {
            // A quiet header row: small, muted text.
            rich_text = rich_text
                .text_style(TextStyle::Small)
                .color(ui.visuals().weak_text_color());
        }
        if let Some(image) = &mut self.image {
            image.alt_text.push(rich_text);
        } else if let Some(block) = &mut self.code_block {
            block.content.push_str(&text);
        } else if let Some(link) = &mut self.link {
            link.text.push(rich_text);
        } else {
            self.push_inline(ui, rich_text);
        }
    }

    fn start_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::Tag,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
    ) {
        match tag {
            pulldown_cmark::Tag::Paragraph => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::Heading { level, id, .. } => {
                let source_id = viewer_id(options);
                let scroll_target = scroll_cache(cache, &source_id)
                    .scroll_to_id_target()
                    .map(str::to_owned);
                if let Some(scroll_target) = scroll_target
                    && let Some(id) = id
                    && id.into_string() == scroll_target
                {
                    ui.scroll_to_cursor(Some(egui::Align::TOP));
                    scroll_cache(cache, &source_id)
                        .scroll_to_id_target_mut()
                        .take();
                }

                // A heading starts a new row. As the first event of a show
                // call (a document, or one block of Terminator's virtualized
                // preview) there is no row to end, and the newline would
                // only add an empty row above the heading.
                if !self.line.should_not_start_newline_forced {
                    newline(ui);
                }
                self.heading_rect = None;
                self.text_style.heading = Some(match level {
                    HeadingLevel::H1 => 0,
                    HeadingLevel::H2 => 1,
                    HeadingLevel::H3 => 2,
                    HeadingLevel::H4 => 3,
                    HeadingLevel::H5 => 4,
                    HeadingLevel::H6 => 5,
                });
            }

            // deliberately not using the built in alerts from pulldown-cmark as
            // the markdown itself cannot be localized :( e.g: [!TIP]
            pulldown_cmark::Tag::BlockQuote(_) => {
                self.is_blockquote = true;
            }
            pulldown_cmark::Tag::CodeBlock(c) => {
                match c {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                        self.code_block = Some(crate::CodeBlock {
                            lang: Some(lang.to_string()),
                            content: String::new(),
                            chunks: Vec::new(),
                        });
                    }
                    pulldown_cmark::CodeBlockKind::Indented => {
                        self.code_block = Some(crate::CodeBlock {
                            lang: None,
                            content: String::new(),
                            chunks: Vec::new(),
                        });
                    }
                }
                self.line.try_insert_start(ui);
            }

            pulldown_cmark::Tag::List(point) => {
                if !self.list.is_inside_a_list() && self.line.can_insert_start() {
                    newline(ui);
                }

                if let Some(number) = point {
                    self.list.start_level_with_number(number);
                } else {
                    self.list.start_level_without_number();
                }
                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
            }

            pulldown_cmark::Tag::Item => {
                self.is_list_item = true;
                self.list
                    .start_item(ui, options, std::mem::take(&mut self.item_is_task));
            }

            pulldown_cmark::Tag::FootnoteDefinition(note) => {
                self.line.try_insert_start(ui);

                self.line.should_start_newline = false;
                self.line.should_end_newline = false;
                footnote(ui, &note);
            }
            pulldown_cmark::Tag::Table(_) => {
                self.is_table = true;
            }
            pulldown_cmark::Tag::TableHead => {}
            pulldown_cmark::Tag::TableRow => {}
            pulldown_cmark::Tag::TableCell => {}
            pulldown_cmark::Tag::Emphasis => {
                self.text_style.emphasis = true;
            }
            pulldown_cmark::Tag::Strong => {
                self.text_style.strong = true;
            }
            pulldown_cmark::Tag::Strikethrough => {
                self.text_style.strikethrough = true;
            }
            pulldown_cmark::Tag::Link { dest_url, .. } => {
                self.link = Some(crate::Link {
                    destination: dest_url.to_string(),
                    text: Vec::new(),
                    chunks: Vec::new(),
                });
            }
            pulldown_cmark::Tag::Image { dest_url, .. } => {
                self.image = Some(crate::Image::new(&dest_url, options));
            }
            pulldown_cmark::Tag::HtmlBlock => {
                self.line.try_insert_start(ui);
            }
            pulldown_cmark::Tag::MetadataBlock(_) => {}

            pulldown_cmark::Tag::DefinitionList => {
                self.line.try_insert_start(ui);
                self.def_list.is_first_item = true;
            }
            pulldown_cmark::Tag::DefinitionListTitle => {
                // we disable newline as the first title should not insert a newline
                // as we have already done that upon the DefinitionList Tag
                if !self.def_list.is_first_item {
                    self.line.try_insert_start(ui)
                } else {
                    self.def_list.is_first_item = false;
                }
            }
            pulldown_cmark::Tag::DefinitionListDefinition => {
                self.def_list.is_def_list_def = true;
            }
            // Not yet supported
            pulldown_cmark::Tag::Superscript | pulldown_cmark::Tag::Subscript => {}
        }
    }

    fn end_tag(
        &mut self,
        ui: &mut Ui,
        tag: pulldown_cmark::TagEnd,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        match tag {
            pulldown_cmark::TagEnd::Paragraph => {
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Heading { .. } => {
                self.line.try_insert_end(ui);
                // GitHub-style divider under H1 and H2, painted in the
                // paragraph gap so the layout does not move.
                if self.text_style.heading.is_some_and(|level| level <= 1)
                    && let Some(rect) = self.heading_rect.take()
                {
                    let y = rect.bottom() + ui.spacing().item_spacing.y * 0.5;
                    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
                    ui.painter().hline(ui.max_rect().x_range(), y, stroke);
                }
                self.text_style.heading = None;
            }
            pulldown_cmark::TagEnd::BlockQuote(_) => {}
            pulldown_cmark::TagEnd::CodeBlock => {
                self.end_code_block(ui, cache, options, max_width);
            }

            pulldown_cmark::TagEnd::List(_) => {
                if self.list.is_last_level() {
                    self.line.should_start_newline = true;
                    self.line.should_end_newline = true;
                }

                self.list.end_level(ui, self.line.can_insert_end());

                if !self.list.is_inside_a_list() {
                    // Reset all the state and make it ready for the next list that occurs
                    self.list = List::default();
                }
            }
            pulldown_cmark::TagEnd::Item => {}
            pulldown_cmark::TagEnd::FootnoteDefinition => {
                self.line.should_start_newline = true;
                self.line.should_end_newline = true;
                self.line.try_insert_end(ui);
            }
            pulldown_cmark::TagEnd::Table => {}
            pulldown_cmark::TagEnd::TableHead => {}
            pulldown_cmark::TagEnd::TableRow => {}
            // The table row layout spaces the columns.
            pulldown_cmark::TagEnd::TableCell => {}
            pulldown_cmark::TagEnd::Emphasis => {
                self.text_style.emphasis = false;
            }
            pulldown_cmark::TagEnd::Strong => {
                self.text_style.strong = false;
            }
            pulldown_cmark::TagEnd::Strikethrough => {
                self.text_style.strikethrough = false;
            }
            pulldown_cmark::TagEnd::Link => {
                if let Some(link) = self.link.take() {
                    let _ = link.end(
                        ui,
                        cache,
                        options,
                        &mut self.deferred_scroll_to_heading,
                        false,
                        0.0,
                    );
                }
            }
            pulldown_cmark::TagEnd::Image => {
                if let Some(image) = self.image.take() {
                    let _ = image.end(ui, cache, options, false, 0.0);
                }
            }
            pulldown_cmark::TagEnd::HtmlBlock => {
                if let Some(html_fn) = options.html_fn {
                    html_fn(ui, &self.html_block);
                    self.html_block.clear();
                }
            }

            pulldown_cmark::TagEnd::MetadataBlock(_) => {}

            pulldown_cmark::TagEnd::DefinitionList => self.line.try_insert_end(ui),
            pulldown_cmark::TagEnd::DefinitionListTitle
            | pulldown_cmark::TagEnd::DefinitionListDefinition => {}
            pulldown_cmark::TagEnd::Superscript | pulldown_cmark::TagEnd::Subscript => {}
        }
    }

    fn end_code_block(
        &mut self,
        ui: &mut Ui,
        cache: &mut CommonMarkCache,
        options: &CommonMarkOptions,
        max_width: f32,
    ) {
        if let Some(block) = self.code_block.take() {
            let _ = block.end(ui, cache, options, max_width, false, 0.0);
            self.line.try_insert_end(ui);
        }
    }
}
