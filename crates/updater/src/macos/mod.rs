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
use objc2_foundation::{NSBundle, NSObject, NSObjectProtocol, NSString, NSUserDefaults, ns_string};
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
            // SAFETY: Sparkle calls this with the SUAppcastItem. `versionString` is its NSString.
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
            // SAFETY: `controller` is the SPUStandardUpdaterController we created.
            // `checkForUpdates:` takes a nil sender.
            unsafe {
                let _: () = msg_send![&*controller, checkForUpdates: Option::<&AnyObject>::None];
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
        // SAFETY: `controller` is our SPUStandardUpdaterController. `updater` is its
        // SUUpdater, and `canCheckForUpdates` returns a Bool.
        unsafe {
            let updater: *mut AnyObject = msg_send![&*controller, updater];
            let can: Bool = msg_send![updater, canCheckForUpdates];
            can.as_bool()
        }
    }
}
mod status_button;
mod termination;
use status_button::keep_sparkle_status_button_in_layout;
#[cfg(feature = "test-support")]
pub use termination::fixture_native_quit;
use termination::{CONTEXT, ORIGINAL, terminate};
pub use termination::{
    cancel_termination, complete_termination, termination_cancelled, termination_requested,
};

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
        if let Some(method) = NSApplication::class().instance_method(sel!(terminate:)) {
            // SAFETY: Runs once on the main thread. Replaces `-[NSApplication terminate:]`
            // with `terminate`, which has that method's signature, and keeps the previous IMP.
            ORIGINAL.get_or_init(|| unsafe {
                method.set_implementation(std::mem::transmute::<
                    unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject),
                    Imp,
                >(terminate))
            });
        }
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
        // SAFETY: `controller` is our SPUStandardUpdaterController and `updater` is its
        // SUUpdater. The selectors match Sparkle and the Bool is only read as a bool.
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
            // SAFETY: `controller` is our SPUStandardUpdaterController. `updater` is its
            // live SUUpdater for this call. The selectors match Sparkle's updater API.
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
                egui::Id::unique(("fixture-target", "check-for-updates")),
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
    // SAFETY: The receiver is a freshly allocated UpdateDelegate with ivars set,
    // on the main thread. `init` is NSObject's initializer and returns a retained object.
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
    // SAFETY: `framework` is Sparkle.framework inside the app bundle. `load` initializes it; failure returns false.
    if !unsafe { framework.load() } {
        return Err(MISSING_UPDATES.into());
    }
    // SAFETY: Sparkle is loaded and `SPUStandardUpdaterController` is its public class.
    // `alloc` and `initWithStartingUpdater:updaterDelegate:userDriverDelegate:` match that
    // class. A null controller is discarded. Later selectors are SUUpdater methods.
    // `delegate` stays retained by the caller.
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
        keep_sparkle_status_button_in_layout();
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
            // SAFETY: `item` is a live menu item. `target` is our UpdateDelegate, which implements `checkForUpdates:`.
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(checkForUpdates:)));
            }
            return true;
        }
    }
    // SAFETY: Allocates an NSMenuItem on the main thread. The initializer returns a retained item.
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            title,
            Some(sel!(checkForUpdates:)),
            &NSString::new(),
        )
    };
    // SAFETY: `item` is the menu item just created. `target` implements `checkForUpdates:`.
    unsafe {
        item.setTarget(Some(target));
    }
    menu.insertItem_atIndex(&item, isize::from(menu.numberOfItems() > 0));
    true
}
