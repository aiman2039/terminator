//! What the bridge knows about wprsd's surfaces, and the window messages
//! each wprsd request causes.

use std::collections::HashMap;

use wprs::serialization::wayland::{
    Buffer, BufferAssignment, BufferData, BufferFormat, Role, SurfaceRequest,
    SurfaceRequestPayload, SurfaceState, WlSurfaceId,
};
use wprs::serialization::xdg_shell::{PopupRequestPayload, ToplevelRequestPayload, XdgPositioner};
use wprs::serialization::{ClientId, RecvType, Request};
use wprs::vec4u8::Vec4u8s;

use crate::compose::{self, Canvas, Layer, Positioner};
use crate::protocol::{FrameEncoding, Framed, HostMessage, Rect, WindowId, WindowRole};

pub type SurfaceKey = (ClientId, WlSurfaceId);
pub type Outgoing = Vec<Framed<HostMessage>>;

/// zstd level for frames: fast; the SSH link is the limit, not the CPU.
const ZSTD_LEVEL: i32 = 1;
/// Deepest subsurface tree drawn; stops cycles from a confused peer.
const MAX_DEPTH: usize = 16;

struct Pixels {
    width: u32,
    height: u32,
    stride: usize,
    opaque: bool,
    data: Vec<u8>,
}

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
}

#[derive(Default)]
pub struct State {
    surfaces: HashMap<SurfaceKey, Surface>,
    /// Pixels wprsd sent just before the commit that uses them.
    pending: Option<Vec<u8>>,
    by_surface: HashMap<SurfaceKey, WindowId>,
    windows: HashMap<WindowId, Window>,
    next_id: u64,
}

impl State {
    pub fn handle(&mut self, request: RecvType<Request>) -> Outgoing {
        match request {
            RecvType::RawBuffer(bytes) => {
                self.pending = Some(bytes);
                Vec::new()
            }
            RecvType::Object(Request::Surface(request)) => self.surface(request),
            RecvType::Object(Request::Toplevel(request)) => match request.payload {
                ToplevelRequestPayload::Destroyed => self.close((request.client, request.surface)),
                _ => Vec::new(),
            },
            RecvType::Object(Request::Popup(request)) => match request.payload {
                PopupRequestPayload::Destroyed => self.close((request.client, request.surface)),
            },
            RecvType::Object(Request::ClientDisconnected(client)) => self.disconnect(client),
            // Cursor and clipboard come in a later phase.
            RecvType::Object(
                Request::CursorImage(_) | Request::Data(_) | Request::Capabilities(_),
            ) => Vec::new(),
        }
    }

    /// The surface a window message names, for input routing.
    pub fn surface_of(&self, id: WindowId) -> Option<SurfaceKey> {
        self.windows.get(&id).map(|window| window.key)
    }

    fn surface(&mut self, request: SurfaceRequest) -> Outgoing {
        let key = (request.client, request.surface);
        match request.payload {
            SurfaceRequestPayload::Commit(state) => self.commit(key, state),
            SurfaceRequestPayload::Destroyed => {
                let out = self.close(key);
                self.surfaces.remove(&key);
                out
            }
        }
    }

    fn commit(&mut self, key: SurfaceKey, state: SurfaceState) -> Outgoing {
        let pending = self.pending.take();
        let surface = self.surfaces.entry(key).or_default();
        match state.buffer {
            Some(BufferAssignment::New(buffer)) => surface.pixels = pixels(buffer, pending),
            Some(BufferAssignment::Removed) => surface.pixels = None,
            None => {}
        }
        if state.role.is_some() {
            surface.role = state.role;
        }
        surface.children = state
            .z_ordered_children
            .iter()
            .map(|child| (child.id, child.position.x, child.position.y))
            .collect();
        match self.root_of(key) {
            Some(root) => self.show(root),
            None => Vec::new(),
        }
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

    /// Opens, renames and redraws the window for `root`.
    fn show(&mut self, root: SurfaceKey) -> Outgoing {
        let Some(size) = self.surfaces.get(&root).and_then(|s| s.pixels.as_ref()) else {
            return Vec::new();
        };
        let (width, height) = (size.width, size.height);
        let mut out = Vec::new();
        let id = match self.by_surface.get(&root) {
            Some(id) => *id,
            None => match self.open(root) {
                Some((id, opened)) => {
                    out.push(opened);
                    id
                }
                None => return out,
            },
        };
        out.extend(self.rename(id, root));
        let mut canvas = Canvas::new(width, height);
        let top = Node {
            key: root,
            x: 0,
            y: 0,
            depth: 0,
        };
        self.draw_tree(&mut canvas, top);
        if let Ok(payload) = zstd::bulk::compress(canvas.pixels(), ZSTD_LEVEL) {
            let damage = vec![Rect {
                x: 0,
                y: 0,
                width: canvas.width(),
                height: canvas.height(),
            }];
            out.push(Framed::with_payload(
                HostMessage::Frame {
                    id,
                    width: canvas.width(),
                    height: canvas.height(),
                    damage,
                    encoding: FrameEncoding::ZstdBgra,
                },
                payload,
            ));
        }
        out
    }

    fn open(&mut self, root: SurfaceKey) -> Option<(WindowId, Framed<HostMessage>)> {
        let role = match &self.surfaces.get(&root)?.role {
            Some(Role::XdgToplevel(_)) => WindowRole::Toplevel,
            Some(Role::XdgPopup(popup)) => {
                let parent_root = self.root_of((root.0, popup.parent_surface_id))?;
                let parent = *self.by_surface.get(&parent_root)?;
                let (x, y) = compose::popup_position(&positioner(&popup.positioner));
                WindowRole::Popup { parent, x, y }
            }
            _ => return None,
        };
        self.next_id = self.next_id.checked_add(1)?;
        let id = WindowId(self.next_id);
        let (title, app_id) = self.names(root);
        self.by_surface.insert(root, id);
        self.windows.insert(
            id,
            Window {
                key: root,
                title: title.clone(),
                app_id: app_id.clone(),
            },
        );
        let opened = HostMessage::WindowOpened {
            id,
            role,
            title,
            app_id,
        };
        Some((id, Framed::new(opened)))
    }

    fn rename(&mut self, id: WindowId, root: SurfaceKey) -> Option<Framed<HostMessage>> {
        let (title, app_id) = self.names(root);
        let window = self.windows.get_mut(&id)?;
        if window.title == title && window.app_id == app_id {
            return None;
        }
        window.title.clone_from(&title);
        window.app_id.clone_from(&app_id);
        Some(Framed::new(HostMessage::WindowChanged {
            id,
            title,
            app_id,
        }))
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

    fn draw_tree(&self, canvas: &mut Canvas, node: Node) {
        let Node { key, x, y, depth } = node;
        let Some(surface) = self.surfaces.get(&key) else {
            return;
        };
        if let Some(pixels) = &surface.pixels {
            canvas.draw(&Layer {
                x,
                y,
                width: pixels.width,
                height: pixels.height,
                stride: pixels.stride,
                pixels: &pixels.data,
                opaque: pixels.opaque,
            });
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
                self.draw_tree(canvas, child);
            }
        }
    }

    fn close(&mut self, key: SurfaceKey) -> Outgoing {
        match self.by_surface.remove(&key) {
            Some(id) => {
                self.windows.remove(&id);
                vec![Framed::new(HostMessage::WindowClosed { id })]
            }
            None => Vec::new(),
        }
    }

    fn disconnect(&mut self, client: ClientId) -> Outgoing {
        let gone: Vec<SurfaceKey> = self
            .surfaces
            .keys()
            .filter(|key| key.0 == client)
            .copied()
            .collect();
        let mut out = Vec::new();
        for key in gone {
            out.extend(self.close(key));
            self.surfaces.remove(&key);
        }
        out
    }
}

/// Decodes a wprs buffer. wprsd sends the bytes in a raw-buffer message just
/// before the commit, filtered for compression; `unfilter` restores BGRA.
fn pixels(buffer: Buffer, pending: Option<Vec<u8>>) -> Option<Pixels> {
    let filtered = match buffer.data {
        BufferData::External => pending?,
        BufferData::Uncompressed(data) => data.0.into(),
        BufferData::Compressed(_) => return None,
    };
    let meta = buffer.metadata;
    let width = u32::try_from(meta.width).ok()?;
    let height = u32::try_from(meta.height).ok()?;
    let stride = usize::try_from(meta.stride).ok()?;
    let needed = stride.checked_mul(usize::try_from(height).ok()?)?;
    if filtered.len() < needed || !filtered.len().is_multiple_of(4) {
        return None;
    }
    let mut data = vec![0; filtered.len()];
    wprs::filtering::unfilter(&Vec4u8s::from(filtered), &mut data);
    Some(Pixels {
        width,
        height,
        stride,
        opaque: matches!(meta.format, BufferFormat::Xrgb8888),
        data,
    })
}

fn positioner(p: &XdgPositioner) -> Positioner {
    Positioner {
        width: p.width,
        height: p.height,
        anchor_x: p.anchor_rect.loc.x,
        anchor_y: p.anchor_rect.loc.y,
        anchor_width: p.anchor_rect.size.w,
        anchor_height: p.anchor_rect.size.h,
        anchor: p.anchor_edges,
        gravity: p.gravity,
        offset_x: p.offset.x,
        offset_y: p.offset.y,
    }
}
