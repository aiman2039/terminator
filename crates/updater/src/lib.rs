//! Sparkle is loaded only from a configured app bundle. Development launches
//! never consult the production feed. NSApplication's delegate remains winit's.
//! The application menu always has Check for Updates…; Sparkle is not required
//! for that item to exist. The macOS menu-bar status item lives in this crate
//! too, because the app crate forbids unsafe.
//!
//! AppKit method swizzling keeps `unsafe` in this crate. Other Terminator crates forbid it.
#[cfg(any(target_os = "macos", test))]
mod schedule;

#[cfg(any(target_os = "macos", test))]
struct FeedEligibility {
    isolated: bool,
    fixture: bool,
    debug: bool,
    bundled: bool,
    has_keys: bool,
}

#[cfg(any(target_os = "macos", test))]
fn skip_production_feed(input: FeedEligibility) -> bool {
    let FeedEligibility {
        isolated,
        fixture,
        debug,
        bundled,
        has_keys,
    } = input;
    !fixture && (isolated || debug || !bundled || !has_keys)
}

#[cfg(any(target_os = "macos", test))]
fn default_automatic_checks(sparkle_choice: Option<bool>) -> bool {
    sparkle_choice.unwrap_or(true)
}
#[cfg(target_os = "macos")]
mod macos {
    const DEVELOPMENT_UPDATES: &str =
        "This development build does not include updates. Install a published Terminator release.";
    const MISSING_UPDATES: &str =
        "The update component is missing. Reinstall Terminator from its release download.";
    const FAILED_UPDATES: &str = "The update component could not be initialized.";

    use eframe::egui;
    use objc2::{
        ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
        rc::Retained,
        runtime::{AnyClass, AnyObject, Bool, Imp, Sel},
        sel,
    };
    use objc2_app_kit::{NSAlert, NSApplication, NSMenuItem};
    use objc2_foundation::{
        NSBundle, NSObject, NSObjectProtocol, NSString, NSUserDefaults, ns_string,
    };
    use std::{
        cell::{Cell, RefCell},
        time::Instant,
    };

    struct UpdateIvars {
        schedule: RefCell<super::schedule::UpdateSchedule>,
        controller: RefCell<Option<Retained<AnyObject>>>,
        unavailable: RefCell<Option<String>>,
    }

    define_class!(
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = UpdateIvars]
        struct UpdateDelegate;
        unsafe impl NSObjectProtocol for UpdateDelegate {}
        impl UpdateDelegate {
            #[unsafe(method(updater:didFindValidUpdate:))]
            fn found(&self, _: &AnyObject, item: &AnyObject) {
                let version: Retained<NSString> = unsafe { msg_send![item, versionString] };
                self.ivars().schedule.borrow_mut().found(version.to_string());
            }
            #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
            fn finished(&self, _: &AnyObject, _: isize, error: Option<&AnyObject>) {
                self.ivars().schedule.borrow_mut().finished(error.is_some());
            }
            #[unsafe(method(checkForUpdates:))]
            fn check_for_updates(&self, _: Option<&AnyObject>) {
                self.request_check();
            }
            #[unsafe(method(validateMenuItem:))]
            fn validate_menu_item(&self, item: &NSMenuItem) -> bool {
                item.action() != Some(sel!(checkForUpdates:)) || self.can_check()
            }
            #[unsafe(method(updaterShouldPromptForPermissionToCheckForUpdates:))]
            fn permission_prompt(&self, _: &AnyObject) -> bool {
                false
            }
        }
    );
    impl UpdateDelegate {
        fn request_check(&self) {
            let controller = self.ivars().controller.borrow().clone();
            if let Some(controller) = controller {
                unsafe {
                    let _: () =
                        msg_send![&*controller, checkForUpdates: Option::<&AnyObject>::None];
                }
                return;
            }
            let reason = self
                .ivars()
                .unavailable
                .borrow()
                .clone()
                .unwrap_or_else(|| DEVELOPMENT_UPDATES.into());
            let alert = NSAlert::new(MainThreadMarker::from(self));
            alert.setMessageText(ns_string!("Updates unavailable"));
            alert.setInformativeText(&NSString::from_str(reason.as_str()));
            alert.runModal();
        }
        fn can_check(&self) -> bool {
            let Some(controller) = self.ivars().controller.borrow().clone() else {
                return true;
            };
            unsafe {
                let updater: *mut AnyObject = msg_send![&*controller, updater];
                let can: Bool = msg_send![updater, canCheckForUpdates];
                can.as_bool()
            }
        }
    }
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
        menu_installed: Cell<bool>,
        // NSBundle must stay loaded for every retained Sparkle object.
        _framework: Option<Retained<NSBundle>>,
    }
    impl Updater {
        pub fn new(ctx: &egui::Context) -> Self {
            let mut result = Self {
                controller: None,
                delegate: None,
                started: Instant::now(),
                menu_installed: Cell::new(false),
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
            let delegate = new_delegate(mtm);
            match load_sparkle(&delegate) {
                Ok((controller, framework)) => {
                    *delegate.ivars().controller.borrow_mut() = Some(controller.clone());
                    result.controller = Some(controller);
                    result._framework = Some(framework);
                }
                Err(reason) => {
                    *delegate.ivars().unavailable.borrow_mut() = Some(reason);
                }
            }
            result.delegate = Some(delegate);
            result.ensure_menu();
            result
        }
        fn ensure_menu(&self) {
            if self.menu_installed.get() {
                return;
            }
            let (Some(delegate), Some(mtm)) = (&self.delegate, MainThreadMarker::new()) else {
                return;
            };
            if install_menu(mtm, delegate) {
                self.menu_installed.set(true);
            }
        }
        pub fn poll(&self) {
            self.ensure_menu();
            let (Some(controller), Some(delegate)) = (&self.controller, &self.delegate) else {
                return;
            };
            let enabled = NSUserDefaults::standardUserDefaults()
                .boolForKey(ns_string!("TerminatorAutomaticUpdateChecks"));
            unsafe {
                let updater: *mut AnyObject = msg_send![controller, updater];
                let available: Bool = msg_send![updater, canCheckForUpdates];
                let action = delegate.ivars().schedule.borrow_mut().next_action(
                    self.started.elapsed(),
                    enabled,
                    available.as_bool(),
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
        #[cfg(feature = "test-support")]
        pub fn menu_installed(&self) -> bool {
            self.menu_installed.get()
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
                            delegate.ivars().schedule.borrow_mut().reset();
                        }
                    }
                    if ui
                        .checkbox(&mut downloads, "Download updates in the background")
                        .changed()
                    {
                        let _: () = msg_send![updater, setAutomaticallyDownloadsUpdates: downloads];
                    }
                }
                ui.label("You can restart to install, or install when you quit. Running sessions stay in the background.");
            } else {
                ui.label(
                    self.delegate
                        .as_ref()
                        .and_then(|delegate| delegate.ivars().unavailable.borrow().clone())
                        .unwrap_or_else(|| DEVELOPMENT_UPDATES.into()),
                );
            }
            let check = ui.button("Check for Updates…");
            #[cfg(feature = "test-support")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    egui::Id::new(("fixture-target", "check-for-updates")),
                    check.rect,
                );
            });
            if check.clicked()
                && let Some(delegate) = &self.delegate
            {
                delegate.request_check();
            }
            ui.hyperlink_to(
                "GitHub release notes",
                "https://github.com/aiman2039/terminator/releases",
            );
        }
    }

    fn new_delegate(mtm: MainThreadMarker) -> Retained<UpdateDelegate> {
        unsafe {
            msg_send![
                super(UpdateDelegate::alloc(mtm).set_ivars(UpdateIvars {
                    schedule: RefCell::new(super::schedule::UpdateSchedule::default()),
                    controller: RefCell::new(None),
                    unavailable: RefCell::new(None),
                })),
                init
            ]
        }
    }

    fn load_sparkle(
        delegate: &UpdateDelegate,
    ) -> Result<(Retained<AnyObject>, Retained<NSBundle>), String> {
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
        let bundled = bundle
            .bundleIdentifier()
            .is_some_and(|s| s.to_string() == "dev.terminator.app");
        let has_keys = bundle
            .objectForInfoDictionaryKey(ns_string!("SUPublicEDKey"))
            .is_some()
            && bundle
                .objectForInfoDictionaryKey(ns_string!("SUFeedURL"))
                .is_some();
        if super::skip_production_feed(super::FeedEligibility {
            isolated,
            fixture,
            debug: cfg!(debug_assertions),
            bundled,
            has_keys,
        }) {
            return Err(DEVELOPMENT_UPDATES.into());
        }
        // Fixture feeds must be baked into a separately identified bundle;
        // environment variables cannot redirect a production updater.
        if !has_keys {
            return Err(DEVELOPMENT_UPDATES.into());
        }
        let Some(path) = bundle.privateFrameworksPath() else {
            return Err(MISSING_UPDATES.into());
        };
        let path = path.stringByAppendingString(ns_string!("/Sparkle.framework"));
        let Some(framework) = NSBundle::bundleWithPath(&path) else {
            return Err(MISSING_UPDATES.into());
        };
        if !unsafe { framework.load() } {
            return Err(MISSING_UPDATES.into());
        }
        unsafe {
            let Some(class) = AnyClass::get(c"SPUStandardUpdaterController") else {
                return Err(FAILED_UPDATES.into());
            };
            let allocated: *mut AnyObject = msg_send![class, alloc];
            let controller: *mut AnyObject = msg_send![allocated, initWithStartingUpdater: false, updaterDelegate: delegate, userDriverDelegate: std::ptr::null::<AnyObject>()];
            let Some(controller) = Retained::from_raw(controller) else {
                return Err(FAILED_UPDATES.into());
            };
            let defaults = NSUserDefaults::standardUserDefaults();
            let key = ns_string!("TerminatorAutomaticUpdateChecks");
            let updater: *mut AnyObject = msg_send![&*controller, updater];
            if defaults.objectForKey(key).is_none() {
                let old = ns_string!("SUEnableAutomaticChecks");
                let sparkle = defaults.objectForKey(old).map(|_| defaults.boolForKey(old));
                defaults.setBool_forKey(super::default_automatic_checks(sparkle), key);
            }
            // Preserve the old choice above before disabling Sparkle's own timer.
            let _: () = msg_send![updater, setAutomaticallyChecksForUpdates: false];
            let _: () = msg_send![&*controller, startUpdater];
            Ok((controller, framework))
        }
    }

    fn install_menu(mtm: MainThreadMarker, target: &UpdateDelegate) -> bool {
        let Some(menu) = NSApplication::sharedApplication(mtm)
            .mainMenu()
            .and_then(|m| m.itemAtIndex(0))
            .and_then(|i| i.submenu())
        else {
            return false;
        };
        let title = ns_string!("Check for Updates…");
        for index in 0..menu.numberOfItems() {
            if let Some(item) = menu.itemAtIndex(index)
                && item.title().isEqualToString(title)
            {
                unsafe {
                    item.setTarget(Some(target));
                    item.setAction(Some(sel!(checkForUpdates:)));
                }
                return true;
            }
        }
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                title,
                Some(sel!(checkForUpdates:)),
                &NSString::new(),
            )
        };
        unsafe {
            item.setTarget(Some(target));
            menu.insertItem_atIndex(&item, isize::from(menu.numberOfItems() > 0));
        }
        true
    }
}
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod status_item;
#[cfg(target_os = "macos")]
pub use status_item::{sync_status_item, take_status_click};

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
        #[cfg(feature = "test-support")]
        pub fn menu_installed(&self) -> bool {
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

#[cfg(not(target_os = "macos"))]
pub fn sync_status_item(_png: &[u8], _width_pt: f32, _height_pt: f32) {}

#[cfg(not(target_os = "macos"))]
pub fn take_status_click() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eligibility(
        isolated: bool,
        fixture: bool,
        debug: bool,
        bundled: bool,
        has_keys: bool,
    ) -> FeedEligibility {
        FeedEligibility {
            isolated,
            fixture,
            debug,
            bundled,
            has_keys,
        }
    }

    #[test]
    fn development_launches_do_not_use_the_production_feed() {
        assert!(skip_production_feed(eligibility(
            true, false, false, true, true
        )));
        assert!(skip_production_feed(eligibility(
            false, false, true, true, true
        )));
        assert!(skip_production_feed(eligibility(
            false, false, false, false, true
        )));
        assert!(skip_production_feed(eligibility(
            false, false, false, true, false
        )));
        assert!(!skip_production_feed(eligibility(
            true, true, true, false, true
        )));
        assert!(!skip_production_feed(eligibility(
            false, false, false, true, true
        )));
        assert!(!skip_production_feed(eligibility(
            true, true, false, false, false
        )));
    }

    #[test]
    fn automatic_checks_default_on_when_sparkle_has_no_choice() {
        assert!(default_automatic_checks(None));
        assert!(default_automatic_checks(Some(true)));
        assert!(!default_automatic_checks(Some(false)));
    }
}
