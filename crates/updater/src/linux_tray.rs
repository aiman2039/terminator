//! Linux tray icon (StatusNotifierItem over D-Bus, through `ksni`). Same API
//! as the macOS status item. One worker thread owns the D-Bus service; GUI
//! frames only queue commands, so nothing blocks the GUI thread. The tray
//! lives as long as the GUI process. Without a StatusNotifierWatcher (for
//! example GNOME without the AppIndicator extension) the worker ends
//! quietly and later calls do nothing.

use super::{
    StatusMenuItem,
    tray_menu::{EMPTY_TITLE, SHOW_TITLE, TrayRow, rgba_to_argb, tray_rows, tray_tooltip},
};
use eframe::egui;
use ksni::blocking::TrayMethods;
use std::sync::{
    Mutex, OnceLock, PoisonError,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

static CLICKED: AtomicBool = AtomicBool::new(false);
static SELECTION: Mutex<Option<String>> = Mutex::new(None);
static REPAINT: OnceLock<egui::Context> = OnceLock::new();
static WORKER: Mutex<Option<mpsc::Sender<Command>>> = Mutex::new(None);
/// Set when the worker ended (no tray host). No new worker starts.
static STOPPED: AtomicBool = AtomicBool::new(false);

enum Command {
    Icon(Vec<u8>),
    Menu(Vec<StatusMenuItem>),
    Waiting(usize),
}

/// Tray picks wake the GUI, which may be idle when the menu is used.
pub(crate) fn set_repaint(ctx: &egui::Context) {
    let _ = REPAINT.set(ctx.clone());
}

fn wake() {
    if let Some(ctx) = REPAINT.get() {
        ctx.request_repaint();
    }
}

fn send(command: Command) {
    if STOPPED.load(Ordering::Acquire) {
        return;
    }
    let mut slot = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    let sender = slot.get_or_insert_with(start);
    if sender.send(command).is_err() {
        *slot = None;
        STOPPED.store(true, Ordering::Release);
    }
}

fn start() -> mpsc::Sender<Command> {
    let (tx, rx) = mpsc::channel();
    if std::thread::Builder::new()
        .name("terminator-tray".into())
        .spawn(move || run(rx))
        .is_err()
    {
        STOPPED.store(true, Ordering::Release);
    }
    tx
}

fn run(rx: mpsc::Receiver<Command>) {
    let mut tray = Tray {
        icon: Vec::new(),
        rows: tray_rows(&[]),
        waiting: 0,
    };
    // Apply what the first frame queued before D-Bus sees the item.
    let Ok(first) = rx.recv() else {
        return;
    };
    tray.apply(first);
    while let Ok(command) = rx.try_recv() {
        tray.apply(command);
    }
    // No watcher or no D-Bus session: fail quietly.
    let Ok(handle) = tray.spawn() else {
        STOPPED.store(true, Ordering::Release);
        return;
    };
    while let Ok(command) = rx.recv() {
        if handle.update(|tray| tray.apply(command)).is_none() {
            STOPPED.store(true, Ordering::Release);
            return;
        }
    }
}

struct Tray {
    icon: Vec<ksni::Icon>,
    rows: Vec<TrayRow>,
    waiting: usize,
}

impl Tray {
    fn apply(&mut self, command: Command) {
        match command {
            Command::Icon(png) => {
                if let Some(icon) = decode_icon(&png) {
                    self.icon = vec![icon];
                }
            }
            Command::Menu(items) => self.rows = tray_rows(&items),
            Command::Waiting(waiting) => self.waiting = waiting,
        }
    }
}

fn decode_icon(png: &[u8]) -> Option<ksni::Icon> {
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()?
        .to_rgba8();
    Some(ksni::Icon {
        width: i32::try_from(image.width()).ok()?,
        height: i32::try_from(image.height()).ok()?,
        data: rgba_to_argb(image.as_raw()),
    })
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "terminator".into()
    }

    fn title(&self) -> String {
        "Terminator".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn status(&self) -> ksni::Status {
        if self.waiting > 0 {
            ksni::Status::NeedsAttention
        } else {
            ksni::Status::Active
        }
    }

    fn attention_icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.icon.clone()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: tray_tooltip(self.waiting),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        click();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        self.rows.iter().map(menu_item).collect()
    }
}

fn menu_item(row: &TrayRow) -> ksni::MenuItem<Tray> {
    use ksni::menu::StandardItem;
    match row {
        TrayRow::Notice { id, label } => {
            let id = id.clone();
            StandardItem {
                label: label.clone(),
                activate: Box::new(move |_: &mut Tray| select(&id)),
                ..Default::default()
            }
            .into()
        }
        TrayRow::Empty => StandardItem {
            label: EMPTY_TITLE.into(),
            enabled: false,
            ..Default::default()
        }
        .into(),
        TrayRow::Separator => ksni::MenuItem::Separator,
        TrayRow::Show => StandardItem {
            label: SHOW_TITLE.into(),
            activate: Box::new(|_: &mut Tray| click()),
            ..Default::default()
        }
        .into(),
    }
}

fn click() {
    CLICKED.store(true, Ordering::Release);
    wake();
}

fn select(id: &str) {
    *SELECTION.lock().unwrap_or_else(PoisonError::into_inner) = Some(id.to_owned());
    wake();
}

pub fn sync_status_item(png: &[u8], _width_pt: f32, _height_pt: f32) {
    send(Command::Icon(png.to_vec()));
}

pub fn sync_status_waiting(waiting: usize) {
    send(Command::Waiting(waiting));
}

pub fn take_status_click() -> bool {
    CLICKED.swap(false, Ordering::AcqRel)
}

pub fn sync_status_menu(items: &[StatusMenuItem]) {
    send(Command::Menu(items.to_vec()));
}

pub fn take_status_selection() -> Option<String> {
    SELECTION
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
}
