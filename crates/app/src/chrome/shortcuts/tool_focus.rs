//! Keyboard focus for the sidebar tool windows: which sidebar holds the
//! focused widget, and a one-shot claim that moves focus into a tool
//! window when its key is pressed. The state lives in egui temp data,
//! like the terminal marker.
use eframe::egui;

/// The left (Projects) sidebar or the right (context) sidebar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// A request that a tool window takes keyboard focus. `reveal` also asks
/// the Explorer to select the current file (Select In).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolClaim {
    pub reveal: bool,
}

/// A claim that waits for its sidebar to paint.
#[derive(Clone, Copy)]
struct Pending {
    side: Side,
    claim: ToolClaim,
    pass: u64,
}

/// What one sidebar painted: its area and layer, whether it hosts a tool
/// window, and whether keyboard focus was inside it.
#[derive(Clone, Copy)]
struct Painted {
    rect: egui::Rect,
    layer: egui::LayerId,
    tool: bool,
    focused: bool,
    pass: u64,
}

fn slot(name: &'static str, side: Option<Side>) -> egui::Id {
    egui::Id::unique(("tool-window-focus", name, side))
}

/// Made in this pass or the one before.
fn fresh(now: u64, pass: u64) -> bool {
    pass.saturating_add(1) >= now
}

/// Ask the tool window on `side` to take keyboard focus when it paints.
pub fn request_tool_focus(ctx: &egui::Context, side: Side, claim: ToolClaim) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| data.insert_temp(slot("pending", None), Pending { side, claim, pass }));
    ctx.request_repaint();
}

/// Before `side` paints its content: arm a claim aimed at it, and keep
/// the sidebar's own focus target alive while it may hold focus.
pub fn begin_sidebar(ui: &mut egui::Ui, side: Side) {
    let now = ui.ctx().cumulative_pass_nr();
    let pending = ui.data(|data| data.get_temp::<Pending>(slot("pending", None)));
    let armed = pending.filter(|p| p.side == side && fresh(now, p.pass));
    let drop = armed.is_some() || pending.is_some_and(|p| !fresh(now, p.pass));
    ui.data_mut(|data| {
        if drop {
            data.remove::<Pending>(slot("pending", None));
        }
        match armed {
            Some(pending) => {
                data.insert_temp(slot("armed", None), pending.claim);
            }
            None => data.remove::<ToolClaim>(slot("armed", None)),
        }
    });
    let target = slot("target", Some(side));
    if armed.is_some() || ui.memory(|memory| memory.has_focus(target)) {
        ui.interact(
            ui.max_rect(),
            target,
            egui::Sense::focusable_noninteractive(),
        );
    }
}

/// The armed claim, taken by the first widget in the tool window that
/// can hold keyboard focus.
pub fn take_tool_focus(ctx: &egui::Context) -> Option<ToolClaim> {
    let id = slot("armed", None);
    ctx.data_mut(|data| {
        let claim = data.get_temp::<ToolClaim>(id);
        data.remove::<ToolClaim>(id);
        claim
    })
}

/// One row of a tool window list. The first row on screen takes the
/// armed claim, so the scroll position stays. A row that just took
/// keyboard focus (the key, Tab, or an arrow key) scrolls into view: egui
/// moves focus by arrow keys to rows off screen and does not scroll.
pub fn tool_row_focus(ui: &egui::Ui, row: &egui::Response) {
    if ui.is_rect_visible(row.rect) && take_tool_focus(ui.ctx()).is_some() {
        row.request_focus();
    }
    if !row.has_focus() {
        return;
    }
    // The row focused last; egui's `gained_focus` misses arrow-key moves.
    let last = slot("row", None);
    if ui.data(|data| data.get_temp::<egui::Id>(last)) != Some(row.id) {
        ui.data_mut(|data| data.insert_temp(last, row.id));
        row.scroll_to_me(None);
    }
}

/// After `side` painted: a claim no widget took focuses the sidebar
/// itself, so the key still toggles. Remembers where focus was.
pub fn end_sidebar(ui: &egui::Ui, side: Side, tool: bool) {
    let ctx = ui.ctx();
    if take_tool_focus(ctx).is_some() {
        ctx.memory_mut(|memory| memory.request_focus(slot("target", Some(side))));
    }
    let (rect, layer) = (ui.max_rect(), ui.layer_id());
    let focused = focused_widget(ctx).is_some_and(|focus| inside(focus, rect, layer));
    let painted = Painted {
        rect,
        layer,
        tool,
        focused,
        pass: ctx.cumulative_pass_nr(),
    };
    ctx.data_mut(|data| data.insert_temp(slot("painted", Some(side)), painted));
}

/// The focused widget's area and layer.
fn focused_widget(ctx: &egui::Context) -> Option<(egui::Rect, egui::LayerId)> {
    let id = ctx.memory(egui::Memory::focused)?;
    ctx.read_response(id)
        .map(|response| (response.rect, response.layer_id))
}

/// A widget at `focus` is in the sidebar painted at `rect` on `layer`. A
/// floating window over the sidebar (the Compare picker, Find in Files)
/// is on its own layer, so it is not.
fn inside(
    (focus, focus_layer): (egui::Rect, egui::LayerId),
    rect: egui::Rect,
    layer: egui::LayerId,
) -> bool {
    focus_layer == layer && focus.intersects(rect)
}

fn painted(ctx: &egui::Context, side: Side) -> Option<Painted> {
    let now = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<Painted>(slot("painted", Some(side))))
        .filter(|painted| fresh(now, painted.pass))
}

/// The sidebar that holds the widget with keyboard focus.
pub fn focused_sidebar(ctx: &egui::Context) -> Option<Side> {
    let focus = focused_widget(ctx)?;
    [Side::Left, Side::Right].into_iter().find(|side| {
        painted(ctx, *side).is_some_and(|painted| inside(focus, painted.rect, painted.layer))
    })
}

/// Escape just dropped the keyboard focus a tool window held.
pub fn escaped_tool_window(ctx: &egui::Context) -> bool {
    ctx.memory(egui::Memory::focused).is_none()
        && ctx.input(|input| input.key_pressed(egui::Key::Escape))
        && [Side::Left, Side::Right]
            .into_iter()
            .any(|side| painted(ctx, side).is_some_and(|painted| painted.tool && painted.focused))
}
