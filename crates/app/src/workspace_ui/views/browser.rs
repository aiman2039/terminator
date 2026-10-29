//! Browser pane: Chrome-style toolbar above the native webview overlay.
use super::super::super::*;
use crate::app::OpenFileModeInput;
use crate::browser::nav::{self, NavState};

const BAR_HEIGHT: f32 = 28.0;
const PROGRESS_HEIGHT: f32 = 2.0;

/// What the browser toolbar's controls asked for this frame.
#[derive(Default)]
struct BrowserBar {
    as_text: bool,
    system: bool,
    reload: bool,
    stop: bool,
    back: bool,
    forward: bool,
    go: bool,
}

struct ToolbarInput<'a> {
    ui: &'a mut egui::Ui,
    draft: &'a mut String,
    href: &'a str,
    target: &'a BrowserTarget,
    nav: &'a NavState,
}

impl App {
    /// Tab label: the page title once loaded, else the host or file name.
    pub(crate) fn browser_title(&self, key: &str, target: &BrowserTarget) -> String {
        if let Some(title) = self.browser_host.nav(key).title {
            return title;
        }
        match target {
            BrowserTarget::Url(url) => {
                nav::split_host(url).map_or_else(|| target.title(), |(_, host, _)| host.to_owned())
            }
            BrowserTarget::File(_) => target.title(),
        }
    }

    pub(in crate::workspace_ui) fn browser_view(
        &mut self,
        ui: &mut egui::Ui,
        key: String,
        target: &BrowserTarget,
    ) {
        let href = crate::browser::href(target).unwrap_or_else(|_| target.title());
        let mut draft = self
            .browser_urls
            .remove(&key)
            .unwrap_or_else(|| href.clone());
        if draft.is_empty() {
            draft.clone_from(&href);
        }
        let nav = self.browser_host.nav(&key);
        let bar = Self::browser_toolbar(ToolbarInput {
            ui,
            draft: &mut draft,
            href: &href,
            target,
            nav: &nav,
        });
        Self::progress_line(ui, nav.loading);
        self.apply_browser_bar(&key, target, (draft, bar));
        if let Some(error) = self.browser_host.error(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
            ui.weak("Open in browser to view the page in your system browser.");
        }
        let rect = ui.available_rect_before_wrap();
        let _ = ui.allocate_rect(rect, egui::Sense::hover());
        self.visible_browsers
            .push(crate::browser_host::VisibleBrowser {
                key,
                target: target.clone(),
                rect,
            });
    }

    fn browser_toolbar(input: ToolbarInput<'_>) -> BrowserBar {
        let ToolbarInput {
            ui,
            draft,
            href,
            target,
            nav,
        } = input;
        let mut bar = BrowserBar::default();
        let size = egui::vec2(ui.available_width(), BAR_HEIGHT);
        ui.allocate_ui_with_layout(
            size,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                bar.back = ui
                    .add_enabled_ui(nav.back, |ui| {
                        appearance::sidebar_action(ui, "ArrowLeft", "Back")
                    })
                    .inner
                    .clicked();
                bar.forward = ui
                    .add_enabled_ui(nav.forward, |ui| {
                        appearance::sidebar_action(ui, "ArrowRight", "Forward")
                    })
                    .inner
                    .clicked();
                if nav.loading {
                    bar.stop = appearance::sidebar_action(ui, "X", "Stop loading").clicked();
                } else {
                    bar.reload = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    Self::browser_menu(ui, target, &mut bar);
                    bar.go = address_bar(ui, draft, href);
                });
            },
        );
        bar
    }

    /// Secondary actions live behind one menu, as in Chrome.
    fn browser_menu(ui: &mut egui::Ui, target: &BrowserTarget, bar: &mut BrowserBar) {
        let menu = appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
            if ui.button("Open in system browser").clicked() {
                bar.system = true;
                ui.close();
            }
            if target.file().is_some() && ui.button("Open as text").clicked() {
                bar.as_text = true;
                ui.close();
            }
        });
        let menu = menu.response.on_hover_text("More actions");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "html-open-browser", menu.rect);
        let _ = menu;
    }

    /// Thin accent line while the page loads; keeps the layout stable.
    fn progress_line(ui: &mut egui::Ui, loading: bool) {
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), PROGRESS_HEIGHT),
            egui::Sense::hover(),
        );
        if !loading {
            return;
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "A fraction in 0..1 fits f32"
        )]
        let phase = (ui.input(|input| input.time) * 0.8).fract() as f32;
        let span = rect.width() * 0.3;
        let x = rect.left() + phase * (rect.width() + span) - span;
        let bar = egui::Rect::from_x_y_ranges(
            x.max(rect.left())..=(x + span).min(rect.right()),
            rect.y_range(),
        );
        ui.painter()
            .rect_filled(bar, 0.0, ui.visuals().selection.bg_fill);
        ui.ctx().request_repaint();
    }

    fn apply_browser_bar(
        &mut self,
        key: &str,
        target: &BrowserTarget,
        (draft, bar): (String, BrowserBar),
    ) {
        if bar.go
            && let Ok(next) = BrowserTarget::from_http_url(&draft)
            && next != *target
        {
            self.browser_submit = Some((key.to_owned(), next));
        }
        self.browser_urls.insert(key.to_owned(), draft);
        if bar.back {
            self.browser_host.go_back(key);
        }
        if bar.forward {
            self.browser_host.go_forward(key);
        }
        if bar.stop {
            self.browser_host.stop(key);
        }
        if bar.reload && !self.browser_host.reload_view(key) {
            self.browser_host.drop_view(key);
        }
        if bar.as_text
            && let Some(path) = target.file()
        {
            self.open_file_mode(OpenFileModeInput {
                path: path.into(),
                line: None,
                split: None,
                external: false,
                text: true,
            });
        }
        if bar.system {
            match target {
                BrowserTarget::File(path) => self.open_in_browser(path),
                BrowserTarget::Url(url) => {
                    let _ = self.jobs.send(Job::Browser(url.clone()));
                }
            }
        }
    }
}

/// Rounded omnibox that fills the row: security icon, then the URL with the
/// host emphasized while not editing. Focus selects all; Enter submits and
/// Escape restores the page URL. Returns whether the user submitted.
fn address_bar(ui: &mut egui::Ui, draft: &mut String, href: &str) -> bool {
    let visuals = ui.visuals().clone();
    let width = ui.available_width();
    let frame = egui::Frame::new()
        .fill(visuals.extreme_bg_color)
        .corner_radius(BAR_HEIGHT / 2.0)
        .inner_margin(egui::Margin::symmetric(10, 2));
    let mut submit = false;
    frame.show(ui, |ui| {
        ui.set_width(width - 20.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let (icon, tip) = security(href);
            let (rect, badge) =
                ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
            appearance::paint_centered_icon(appearance::PaintCenteredIconInput {
                ui,
                rect,
                icon,
                size: 13.0,
                tint: appearance::icon_color(),
            });
            let _ = badge.on_hover_text(tip);
            let id = ui.make_persistent_id("browser-url");
            let editing = ui.memory(|memory| memory.has_focus(id));
            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap: f32| {
                let job = url_job(ui, text.as_str(), editing);
                ui.fonts_mut(|fonts| fonts.layout_job(job))
            };
            let mut output = appearance::singleline(draft)
                .id(id)
                .frame(egui::Frame::NONE)
                .desired_width(ui.available_width())
                .hint_text("Search or type a URL")
                .layouter(&mut layouter)
                .show(ui);
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "browser-url", output.response.rect);
            if output.response.gained_focus() {
                output
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::select_all(&output.galley)));
                output.state.store(ui.ctx(), id);
            }
            if output.response.lost_focus() {
                if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
                    href.clone_into(draft);
                } else {
                    submit = ui.input(|input| input.key_pressed(egui::Key::Enter));
                }
            }
        });
    });
    submit
}

fn security(href: &str) -> (&'static str, &'static str) {
    if href.starts_with("https://") {
        ("Lock", "Connection is secure")
    } else if href.starts_with("http://") {
        ("CircleAlert", "Not secure")
    } else {
        ("FileCode", "Local file")
    }
}

/// Host in strong text, scheme and path dimmed, as Chrome shows URLs.
fn url_job(ui: &egui::Ui, text: &str, editing: bool) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let strong = ui.visuals().strong_text_color();
    let weak = ui.visuals().weak_text_color();
    let mut job = egui::text::LayoutJob::default();
    let mut push = |part: &str, color| {
        job.append(part, 0.0, egui::TextFormat::simple(font.clone(), color));
    };
    match nav::split_host(text).filter(|_| !editing) {
        Some((scheme, host, rest)) => {
            push(scheme, weak);
            push(host, strong);
            push(rest, weak);
        }
        None => push(text, ui.visuals().text_color()),
    }
    job
}
