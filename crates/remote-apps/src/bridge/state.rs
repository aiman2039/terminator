//! What the bridge knows about wprsd's surfaces, and the window messages
//! they cause.
//!
//! Commits only record pixels and mark the changed area of their window.
//! [`State::flush`] then renders each changed window once, only inside the
//! changed area, so a burst of commits becomes one small frame per window
//! and the output never runs ahead of the link (the caller flushes only
//! when the writer is idle).

use std::collections::HashMap;

use wprs::serialization::wayland::{
    BufferAssignment, CursorImage, CursorImageStatus, Role, SurfaceRequest, SurfaceRequestPayload,
    SurfaceState, WlSurfaceId,
};
use wprs::serialization::xdg_shell::{PopupRequestPayload, ToplevelRequestPayload};
use wprs::serialization::{ClientId, RecvType, Request};

use super::buffer::{Pixels, pixels, positioner};
use crate::compose::{self, Canvas};
use crate::protocol::{FrameEncoding, Framed, HostMessage, Rect, WindowId, WindowRole};

pub type SurfaceKey = (ClientId, WlSurfaceId);
pub type Outgoing = Vec<Framed<HostMessage>>;

/// zstd level for frames: fast; the SSH link is the limit, not the CPU.
const ZSTD_LEVEL: i32 = 1;
/// Deepest subsurface tree drawn; stops cycles from a confused peer.
const MAX_DEPTH: usize = 16;

#[derive(Default)]
struct Surface {
    pixels: Option<Pixels>,
    role: Option<Role>,
    /// Subsurfaces with their offsets, bottom to top. wprsd also lists the
    /// surface itself; it is skipped when drawing.
    children: Vec<(WlSurfaceId, i32, i32)>,
}

/// A surface to draw at `x`,`y` in window coordinates, `depth` levels below
/// the window's root surface.
#[derive(Clone, Copy)]
struct Node {
    key: SurfaceKey,
    x: i32,
    y: i32,
    depth: usize,
}

struct Window {
    key: SurfaceKey,
    title: String,
    app_id: String,
    /// The composited window, kept so a change redraws only its area.
    canvas: Canvas,
    /// Area changed since the last frame; the whole window after a resize.
    damage: Option<Rect>,
}

#[derive(Default)]
pub struct State {
    surfaces: HashMap<SurfaceKey, Surface>,
    /// Pixels wprsd sent just before the commit that uses them.
    pending: Option<Vec<u8>>,
    by_surface: HashMap<SurfaceKey, WindowId>,
    windows: HashMap<WindowId, Window>,
    /// The surface that holds the cursor image, and its hotspot.
    cursor: Option<(SurfaceKey, (i32, i32))>,
    /// Window, cursor and clipboard messages waiting for the next flush, in
    /// order.
    control: Outgoing,
}

impl State {
    /// Records a wprsd request. Messages come out of [`State::flush`].
    pub fn handle(&mut self, request: RecvType<Request>) {
        match request {
            RecvType::RawBuffer(bytes) => self.pending = Some(bytes),
            RecvType::Object(Request::Surface(request)) => self.surface(request),
            RecvType::Object(Request::Toplevel(request)) => {
                if matches!(request.payload, ToplevelRequestPayload::Destroyed) {
                    self.close((request.client, request.surface));
                }
            }
            RecvType::Object(Request::Popup(request)) => match request.payload {
                PopupRequestPayload::Destroyed => self.close((request.client, request.surface)),
            },
            RecvType::Object(Request::ClientDisconnected(client)) => self.disconnect(client),
            RecvType::Object(Request::CursorImage(image)) => self.cursor_image(image),
            // The clipboard is handled by the bridge loop, not here.
            RecvType::Object(Request::Data(_) | Request::Capabilities(_)) => {}
        }
    }

    /// Queues a message that is not a frame (for example clipboard text).
    pub fn queue(&mut self, message: Framed<HostMessage>) {
        self.control.push(message);
    }

    /// Whether [`State::flush`] has frames to render.
    #[cfg(test)]
    pub fn has_frames(&self) -> bool {
        self.windows.values().any(|w| w.damage.is_some())
    }

    /// Queued control messages, then (when `frames`) one frame per changed
    /// window holding only its changed area.
    pub fn flush(&mut self, frames: bool) -> Outgoing {
        let mut out = std::mem::take(&mut self.control);
        if !frames {
            return out;
        }
        let ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.damage.is_some())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            out.extend(self.render(id));
        }
        out
    }

    fn render(&mut self, id: WindowId) -> Option<Framed<HostMessage>> {
        let window = self.windows.get_mut(&id)?;
        let damage = window.damage.take()?;
        let key = window.key;
        let mut canvas = std::mem::replace(&mut window.canvas, Canvas::new(0, 0));
        let (width, height) = self
            .surfaces
            .get(&key)?
            .pixels
            .as_ref()
            .map(|p| (p.width, p.height))?;
        let damage = if (canvas.width(), canvas.height()) == (width, height) {
            damage
        } else {
            canvas = Canvas::new(width, height);
            canvas.bounds()
        };
        let area = compose::clamp(damage, width, height);
        if let Some(area) = area {
            canvas.clear(area);
            let top = Node {
                key,
                x: 0,
                y: 0,
                depth: 0,
            };
            self.draw_tree(&mut canvas, top, area);
        }
        let payload = area.map(|a| canvas.region(a));
        if let Some(window) = self.windows.get_mut(&id) {
            window.canvas = canvas;
        }
        let area = area?;
        let payload = zstd::bulk::compress(&payload?, ZSTD_LEVEL).ok()?;
        Some(Framed::with_payload(
            HostMessage::Frame {
                id,
                width,
                height,
                damage: vec![area],
                encoding: FrameEncoding::ZstdBgra,
            },
            payload,
        ))
    }

    fn cursor_image(&mut self, image: CursorImage) {
        let named = |name: Option<String>| {
            Framed::new(HostMessage::Cursor {
                width: 0,
                height: 0,
                hotspot_x: 0,
                hotspot_y: 0,
                name,
            })
        };
        match image.status {
            CursorImageStatus::Hidden => {
                self.cursor = None;
                self.control.push(named(None));
            }
            CursorImageStatus::Named(name) => {
                self.cursor = None;
                self.control.push(named(Some(name)));
            }
            CursorImageStatus::Surface {
                client_surface,
                hotspot,
            } => {
                let key = (client_surface.client, client_surface.surface);
                self.cursor = Some((key, (hotspot.x, hotspot.y)));
                self.queue_cursor_pixels();
            }
        }
    }

    /// The cursor surface's pixels, as tightly packed BGRA rows.
    fn queue_cursor_pixels(&mut self) {
        let Some((key, (hotspot_x, hotspot_y))) = self.cursor else {
            return;
        };
        let Some(pixels) = self.surfaces.get(&key).and_then(|s| s.pixels.as_ref()) else {
            return;
        };
        let mut canvas = Canvas::new(pixels.width, pixels.height);
        canvas.draw(&pixels.layer(0, 0));
        self.control.push(Framed::with_payload(
            HostMessage::Cursor {
                width: canvas.width(),
                height: canvas.height(),
                hotspot_x,
                hotspot_y,
                name: None,
            },
            canvas.pixels().to_vec(),
        ));
    }

    /// The surface a window message names, for input routing.
    pub fn surface_of(&self, id: WindowId) -> Option<SurfaceKey> {
        self.windows.get(&id).map(|window| window.key)
    }

    fn surface(&mut self, request: SurfaceRequest) {
        let key = (request.client, request.surface);
        match request.payload {
            SurfaceRequestPayload::Commit(state) => self.commit(key, state),
            SurfaceRequestPayload::Destroyed => {
                // A subsurface going away uncovers part of its window.
                if let Some(root) = self.root_of(key).filter(|root| *root != key) {
                    self.damage_all(root);
                }
                self.close(key);
                self.surfaces.remove(&key);
            }
        }
    }

    fn commit(&mut self, key: SurfaceKey, state: SurfaceState) {
        let pending = self.pending.take();
        let surface = self.surfaces.entry(key).or_default();
        let old_size = surface.pixels.as_ref().map(|p| (p.width, p.height));
        match state.buffer {
            Some(BufferAssignment::New(buffer)) => surface.pixels = pixels(buffer, pending),
            Some(BufferAssignment::Removed) => surface.pixels = None,
            None => {}
        }
        if state.role.is_some() {
            surface.role = state.role;
        }
        let children: Vec<(WlSurfaceId, i32, i32)> = state
            .z_ordered_children
            .iter()
            .map(|child| (child.id, child.position.x, child.position.y))
            .collect();
        let moved = children != surface.children;
        surface.children = children;
        let new_size = surface.pixels.as_ref().map(|p| (p.width, p.height));
        if self.cursor.is_some_and(|(cursor, _)| cursor == key) {
            self.queue_cursor_pixels();
            return;
        }
        let Some(root) = self.root_of(key) else {
            return;
        };
        if !self.ensure_open(root) {
            return;
        }
        // Without damage, after a resize or a subsurface move, redraw all.
        match (state.damage, moved || old_size != new_size) {
            (Some(rects), false) => {
                let (dx, dy) = self.offset_in(key, root);
                for r in rects {
                    self.damage(
                        root,
                        Rect {
                            x: r.loc.x.saturating_add(dx),
                            y: r.loc.y.saturating_add(dy),
                            width: u32::try_from(r.size.w).unwrap_or(0),
                            height: u32::try_from(r.size.h).unwrap_or(0),
                        },
                    );
                }
            }
            _ => self.damage_all(root),
        }
    }

    fn damage(&mut self, root: SurfaceKey, rect: Rect) {
        if let Some(id) = self.by_surface.get(&root)
            && let Some(window) = self.windows.get_mut(id)
        {
            window.damage = Some(match window.damage {
                Some(old) => compose::union(old, rect),
                None => rect,
            });
        }
    }

    fn damage_all(&mut self, root: SurfaceKey) {
        if let Some(full) = self
            .surfaces
            .get(&root)
            .and_then(|s| s.pixels.as_ref())
            .map(Pixels::bounds)
        {
            self.damage(root, full);
        }
    }

    /// Where `key` sits inside its window `root`.
    fn offset_in(&self, key: SurfaceKey, root: SurfaceKey) -> (i32, i32) {
        let (mut x, mut y) = (0_i32, 0_i32);
        let mut current = key;
        for _ in 0..MAX_DEPTH {
            if current == root {
                break;
            }
            let Some(Role::SubSurface(sub)) =
                self.surfaces.get(&current).and_then(|s| s.role.as_ref())
            else {
                break;
            };
            let parent = (current.0, sub.parent);
            if let Some(&(_, dx, dy)) = self
                .surfaces
                .get(&parent)
                .and_then(|p| p.children.iter().find(|(id, _, _)| *id == current.1))
            {
                x = x.saturating_add(dx);
                y = y.saturating_add(dy);
            }
            current = parent;
        }
        (x, y)
    }

    /// The toplevel or popup that `key` belongs to, through subsurface
    /// parents.
    fn root_of(&self, key: SurfaceKey) -> Option<SurfaceKey> {
        let mut current = key;
        for _ in 0..MAX_DEPTH {
            match &self.surfaces.get(&current)?.role {
                Some(Role::XdgToplevel(_) | Role::XdgPopup(_)) => return Some(current),
                Some(Role::SubSurface(sub)) => current = (current.0, sub.parent),
                Some(Role::Cursor(_)) | None => return None,
            }
        }
        None
    }

    /// Opens the window for `root` once it has pixels, and renames it when
    /// its title changed. Returns whether the window is open.
    fn ensure_open(&mut self, root: SurfaceKey) -> bool {
        if self
            .surfaces
            .get(&root)
            .and_then(|s| s.pixels.as_ref())
            .is_none()
        {
            return false;
        }
        match self.by_surface.get(&root).copied() {
            Some(id) => {
                self.rename(id, root);
                true
            }
            None => self.open(root),
        }
    }

    fn open(&mut self, root: SurfaceKey) -> bool {
        let role = match self.surfaces.get(&root).and_then(|s| s.role.as_ref()) {
            Some(Role::XdgToplevel(_)) => WindowRole::Toplevel,
            Some(Role::XdgPopup(popup)) => {
                let Some(parent) = self
                    .root_of((root.0, popup.parent_surface_id))
                    .and_then(|parent_root| self.by_surface.get(&parent_root).copied())
                else {
                    return false;
                };
                let (x, y) = compose::popup_position(&positioner(&popup.positioner));
                WindowRole::Popup { parent, x, y }
            }
            _ => return false,
        };
        // wprsd's surface ID: the same for every bridge that attaches, so a
        // reconnect keeps each window's identity (and its tab).
        let id = WindowId(root.1.0);
        if self.windows.contains_key(&id) {
            return false;
        }
        let (title, app_id) = self.names(root);
        self.by_surface.insert(root, id);
        self.windows.insert(
            id,
            Window {
                key: root,
                title: title.clone(),
                app_id: app_id.clone(),
                canvas: Canvas::new(0, 0),
                damage: None,
            },
        );
        self.control.push(Framed::new(HostMessage::WindowOpened {
            id,
            role,
            title,
            app_id,
        }));
        self.damage_all(root);
        true
    }

    fn rename(&mut self, id: WindowId, root: SurfaceKey) {
        let (title, app_id) = self.names(root);
        let Some(window) = self.windows.get_mut(&id) else {
            return;
        };
        if window.title == title && window.app_id == app_id {
            return;
        }
        window.title.clone_from(&title);
        window.app_id.clone_from(&app_id);
        self.control.push(Framed::new(HostMessage::WindowChanged {
            id,
            title,
            app_id,
        }));
    }

    fn names(&self, root: SurfaceKey) -> (String, String) {
        match self.surfaces.get(&root).and_then(|s| s.role.as_ref()) {
            Some(Role::XdgToplevel(toplevel)) => (
                toplevel.title.clone().unwrap_or_default(),
                toplevel.app_id.clone().unwrap_or_default(),
            ),
            _ => (String::new(), String::new()),
        }
    }

    /// Draws the surface tree of `node` into `area` of `canvas`.
    fn draw_tree(&self, canvas: &mut Canvas, node: Node, area: Rect) {
        let Node { key, x, y, depth } = node;
        let Some(surface) = self.surfaces.get(&key) else {
            return;
        };
        if let Some(pixels) = &surface.pixels {
            canvas.draw_in(&pixels.layer(x, y), area);
        }
        if depth >= MAX_DEPTH {
            return;
        }
        for &(child, dx, dy) in &surface.children {
            if child != key.1 {
                let child = Node {
                    key: (key.0, child),
                    x: x.saturating_add(dx),
                    y: y.saturating_add(dy),
                    depth: depth.saturating_add(1),
                };
                self.draw_tree(canvas, child, area);
            }
        }
    }

    fn close(&mut self, key: SurfaceKey) {
        if let Some(id) = self.by_surface.remove(&key) {
            self.windows.remove(&id);
            self.control
                .push(Framed::new(HostMessage::WindowClosed { id }));
        }
    }

    fn disconnect(&mut self, client: ClientId) {
        let gone: Vec<SurfaceKey> = self
            .surfaces
            .keys()
            .filter(|key| key.0 == client)
            .copied()
            .collect();
        for key in gone {
            self.close(key);
            self.surfaces.remove(&key);
        }
    }
}
