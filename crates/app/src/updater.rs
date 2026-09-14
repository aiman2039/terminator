//! Sparkle is loaded only from a configured app bundle. Development launches
//! never consult the production feed. NSApplication's delegate remains winit's.
#[cfg(target_os = "macos")]
mod macos {
    use eframe::egui;
    use objc2::{
        ClassType, MainThreadMarker, MainThreadOnly, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
        sel,
    };
    use objc2_app_kit::{NSApplication, NSMenuItem};
    use objc2_foundation::{NSBundle, NSString, ns_string};
    use std::sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    };

    static REQUESTED: AtomicBool = AtomicBool::new(false);
    static ORIGINAL: OnceLock<Imp> = OnceLock::new();
    static CANCELLED: AtomicBool = AtomicBool::new(false);
    static CONTEXT: OnceLock<egui::Context> = OnceLock::new();

    unsafe extern "C-unwind" fn terminate(
        _application: *mut AnyObject,
        _sel: Sel,
        _sender: *mut AnyObject,
    ) {
        // Do not enter AppKit's NSTerminateLater modal loop: it prevents winit
        // from applying the queued GUI results needed for our checkpoint.
        REQUESTED.store(true, Ordering::Release);
        if let Some(ctx) = CONTEXT.get() {
            ctx.request_repaint();
        }
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_native_quit() {
        dispatch2::DispatchQueue::main().exec_async(|| {
            NSApplication::sharedApplication(MainThreadMarker::new().expect("GUI main thread"))
                .terminate(None);
        });
    }
    pub fn termination_requested() -> bool {
        REQUESTED.swap(false, Ordering::AcqRel)
    }
    pub fn termination_cancelled() -> bool {
        CANCELLED.swap(false, Ordering::AcqRel)
    }
    pub fn cancel_termination() {
        REQUESTED.store(false, Ordering::Release);
    }
    pub fn complete_termination(_ctx: &egui::Context) {
        if cfg!(test) {
            return;
        }
        // AppKit broadcasts applicationWillTerminate synchronously. Dispatch
        // outside the egui/winit callback to avoid re-entering winit's handler.
        dispatch2::DispatchQueue::main().exec_async(|| {
            let app =
                NSApplication::sharedApplication(MainThreadMarker::new().expect("GUI main thread"));
            if let Some(original) = ORIGINAL.get() {
                // Exact ABI of -[NSApplication terminate:]. Calling the saved
                // implementation preserves AppKit and Sparkle quit observers.
                unsafe {
                    let original: unsafe extern "C-unwind" fn(
                        *const NSApplication,
                        Sel,
                        *const AnyObject,
                    ) = std::mem::transmute(*original);
                    original(&*app, sel!(terminate:), std::ptr::null());
                }
            }
            CANCELLED.store(true, Ordering::Release);
            if let Some(ctx) = CONTEXT.get() {
                ctx.request_repaint();
            }
        });
    }

    pub struct Updater {
        controller: Option<Retained<AnyObject>>,
        // NSBundle must stay loaded for every retained Sparkle object.
        _framework: Option<Retained<NSBundle>>,
    }
    impl Updater {
        pub fn new(ctx: &egui::Context) -> Self {
            let mut result = Self {
                controller: None,
                _framework: None,
            };
            if cfg!(test) {
                return result;
            }
            let Some(mtm) = MainThreadMarker::new() else {
                return result;
            };
            CONTEXT.get_or_init(|| ctx.clone());
            ORIGINAL.get_or_init(|| unsafe {
                let method = NSApplication::class()
                    .instance_method(sel!(terminate:))
                    .expect("NSApplication terminate:");
                method.set_implementation(std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    Imp,
                >(terminate))
            });
            let isolated = std::env::var_os("TERMINATOR_DATA_DIR").is_some()
                || std::env::args().any(|a| a == "--data-dir")
                || std::env::var_os("TERMINATOR_CONFIG_DIR").is_some()
                || std::env::var_os("TERMINATOR_RUNTIME_DIR").is_some();
            let bundle = NSBundle::mainBundle();
            let fixture = cfg!(feature = "test-support")
                && isolated
                && std::env::var_os("TERMINATOR_UPDATE_FIXTURE").is_some()
                && bundle
                    .bundleIdentifier()
                    .is_some_and(|s| s.to_string() == "dev.terminator.update-fixture");
            if !fixture
                && (isolated
                    || cfg!(debug_assertions)
                    || !bundle
                        .bundleIdentifier()
                        .is_some_and(|s| s.to_string() == "dev.terminator.app"))
            {
                return result;
            }
            // Fixture feeds must be baked into a separately identified bundle;
            // environment variables cannot redirect a production updater.
            if bundle
                .objectForInfoDictionaryKey(ns_string!("SUPublicEDKey"))
                .is_none()
                || bundle
                    .objectForInfoDictionaryKey(ns_string!("SUFeedURL"))
                    .is_none()
            {
                return result;
            }
            let Some(path) = bundle.privateFrameworksPath() else {
                return result;
            };
            let path = path.stringByAppendingString(ns_string!("/Sparkle.framework"));
            let Some(framework) = NSBundle::bundleWithPath(&path) else {
                return result;
            };
            if !unsafe { framework.load() } {
                return result;
            }
            unsafe {
                let Some(class) = AnyClass::get(c"SPUStandardUpdaterController") else {
                    return result;
                };
                let allocated: *mut AnyObject = msg_send![class, alloc];
                let controller: *mut AnyObject = msg_send![allocated, initWithStartingUpdater: true, updaterDelegate: std::ptr::null::<AnyObject>(), userDriverDelegate: std::ptr::null::<AnyObject>()];
                let Some(controller) = Retained::from_raw(controller) else {
                    return result;
                };
                let app = NSApplication::sharedApplication(mtm);
                if let Some(menu) = app
                    .mainMenu()
                    .and_then(|m| m.itemAtIndex(0))
                    .and_then(|i| i.submenu())
                {
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        ns_string!("Check for Updates…"),
                        Some(sel!(checkForUpdates:)),
                        &NSString::new(),
                    );
                    item.setTarget(Some(&controller));
                    menu.insertItem_atIndex(&item, 1);
                }
                result.controller = Some(controller);
                result._framework = Some(framework);
            }
            result
        }
        #[cfg(feature = "test-support")]
        pub fn available(&self) -> bool {
            self.controller.is_some()
        }
        pub fn settings(&self, ui: &mut egui::Ui) {
            ui.heading("Updates");
            if let Some(controller) = &self.controller {
                ui.weak("Update preferences take effect immediately.");
                unsafe {
                    let updater: *mut AnyObject = msg_send![controller, updater];
                    let updater = &*updater;
                    let checks: Bool = msg_send![updater, automaticallyChecksForUpdates];
                    let downloads: Bool = msg_send![updater, automaticallyDownloadsUpdates];
                    let mut checks = checks.as_bool();
                    let mut downloads = downloads.as_bool();
                    if ui
                        .checkbox(&mut checks, "Check for updates automatically")
                        .changed()
                    {
                        let _: () = msg_send![updater, setAutomaticallyChecksForUpdates: checks];
                    }
                    if ui
                        .checkbox(&mut downloads, "Download updates in the background")
                        .changed()
                    {
                        let _: () = msg_send![updater, setAutomaticallyDownloadsUpdates: downloads];
                    }
                    if ui.button("Check for Updates…").clicked() {
                        let _: () =
                            msg_send![controller, checkForUpdates: std::ptr::null::<AnyObject>()];
                    }
                }
                ui.label("You can restart to install, or install when you quit. Running sessions stay in the background.");
            } else {
                ui.label("In-app updates are available in installed macOS releases.");
            }
            ui.hyperlink_to(
                "GitHub release notes",
                "https://github.com/aiman2039/terminator/releases",
            );
        }
    }
}
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(not(target_os = "macos"))]
mod other {
    use eframe::egui;
    pub struct Updater;
    impl Updater {
        pub fn new(_: &egui::Context) -> Self {
            Self
        }
        #[cfg(feature = "test-support")]
        pub fn available(&self) -> bool {
            false
        }
        pub fn settings(&self, ui: &mut egui::Ui) {
            ui.heading("Updates");
            ui.hyperlink_to(
                "Download releases",
                "https://github.com/aiman2039/terminator/releases",
            );
        }
    }
    #[cfg(feature = "test-support")]
    pub fn fixture_native_quit() {}
    pub fn termination_requested() -> bool {
        false
    }
    pub fn termination_cancelled() -> bool {
        false
    }
    pub fn cancel_termination() {}
    pub fn complete_termination(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
#[cfg(not(target_os = "macos"))]
pub use other::*;
