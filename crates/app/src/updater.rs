//! Sparkle is loaded only from a configured app bundle. Development launches
//! never consult the production feed. NSApplication's delegate remains winit's.
#[cfg(any(target_os = "macos", test))]
mod schedule;
#[cfg(target_os = "macos")]
mod macos {
    use eframe::egui;
    use objc2::{
        ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
        sel,
    };
    use objc2_app_kit::{NSApplication, NSMenuItem};
    use objc2_foundation::{
        NSBundle, NSObject, NSObjectProtocol, NSString, NSUserDefaults, ns_string,
    };
    use std::{cell::RefCell, time::Instant};

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = RefCell<super::schedule::UpdateSchedule>]
        struct UpdateDelegate;
        unsafe impl NSObjectProtocol for UpdateDelegate {}
        impl UpdateDelegate {
            #[unsafe(method(updater:didFindValidUpdate:))]
            fn found(&self, _: &AnyObject, item: &AnyObject) {
                let version: Retained<NSString> = unsafe { msg_send![item, versionString] };
                self.ivars().borrow_mut().found(version.to_string());
            }
            #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
            fn finished(&self, _: &AnyObject, _: isize, error: Option<&AnyObject>) {
                self.ivars().borrow_mut().finished(error.is_some());
            }
        }
    );
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
        delegate: Option<Retained<UpdateDelegate>>,
        started: Instant,
        // NSBundle must stay loaded for every retained Sparkle object.
        _framework: Option<Retained<NSBundle>>,
    }
    impl Updater {
        pub fn new(ctx: &egui::Context) -> Self {
            let mut result = Self {
                controller: None,
                delegate: None,
                started: Instant::now(),
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
                let delegate: Retained<UpdateDelegate> = msg_send![
                    super(
                        UpdateDelegate::alloc(mtm)
                            .set_ivars(RefCell::new(super::schedule::UpdateSchedule::default()))
                    ),
                    init
                ];
                let allocated: *mut AnyObject = msg_send![class, alloc];
                let controller: *mut AnyObject = msg_send![allocated, initWithStartingUpdater: false, updaterDelegate: &*delegate, userDriverDelegate: std::ptr::null::<AnyObject>()];
                let Some(controller) = Retained::from_raw(controller) else {
                    return result;
                };
                let defaults = NSUserDefaults::standardUserDefaults();
                let key = ns_string!("TerminatorAutomaticUpdateChecks");
                let updater: *mut AnyObject = msg_send![&*controller, updater];
                if defaults.objectForKey(key).is_none() {
                    let enabled: Bool = msg_send![updater, automaticallyChecksForUpdates];
                    defaults.setBool_forKey(enabled.as_bool(), key);
                }
                // Preserve the old choice above before disabling Sparkle's own timer.
                let _: () = msg_send![updater, setAutomaticallyChecksForUpdates: false];
                let _: () = msg_send![&*controller, startUpdater];
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
                result.delegate = Some(delegate);
                result._framework = Some(framework);
            }
            result
        }
        pub fn poll(&self) {
            let (Some(controller), Some(delegate)) = (&self.controller, &self.delegate) else {
                return;
            };
            let enabled = NSUserDefaults::standardUserDefaults()
                .boolForKey(ns_string!("TerminatorAutomaticUpdateChecks"));
            unsafe {
                let updater: *mut AnyObject = msg_send![controller, updater];
                let busy: Bool = msg_send![updater, sessionInProgress];
                let action = delegate.ivars().borrow_mut().next_action(
                    self.started.elapsed(),
                    enabled,
                    busy.as_bool(),
                );
                // Release the RefCell before invoking APIs which can call the delegate.
                match action {
                    super::schedule::Action::Probe => {
                        let _: () = msg_send![updater, checkForUpdateInformation];
                    }
                    super::schedule::Action::Offer => {
                        let _: () = msg_send![updater, checkForUpdatesInBackground];
                    }
                    super::schedule::Action::None => {}
                }
            }
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
                    let defaults = NSUserDefaults::standardUserDefaults();
                    let key = ns_string!("TerminatorAutomaticUpdateChecks");
                    let downloads: Bool = msg_send![updater, automaticallyDownloadsUpdates];
                    let mut checks = defaults.boolForKey(key);
                    let mut downloads = downloads.as_bool();
                    if ui
                        .checkbox(&mut checks, "Check for updates every minute")
                        .changed()
                    {
                        defaults.setBool_forKey(checks, key);
                        if let Some(delegate) = &self.delegate {
                            delegate.ivars().borrow_mut().reset();
                        }
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
        pub fn poll(&self) {}
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
