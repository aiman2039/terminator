#![forbid(unsafe_code)]
//! Startup only: preflight, paths, crash install, single-instance lock,
//! daemon handshake, then eframe. The application lives in the library.
use anyhow::{Context, Result};
use eframe::egui;
use fs2::FileExt;
use std::fs;
use terminator::{App, daemon_connection, installation};
use terminator_core::{Paths, crash, generations};
mod renderer;

/// Fixture runs render on a schedule while occluded; without a held
/// activity macOS App Nap suspends their timers mid-case (stalled
/// screenshots, windows that never register). Test builds only.
#[cfg(all(target_os = "macos", feature = "test-support"))]
fn hold_fixture_activity() {
    use std::sync::OnceLock;
    if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_none() {
        return;
    }
    // Leaked: the activity must stay alive for the whole process.
    static ACTIVITY: OnceLock<()> = OnceLock::new();
    ACTIVITY.get_or_init(|| {
        let reason = objc2_foundation::NSString::from_str("fixture rendering");
        let activity = objc2_foundation::NSProcessInfo::processInfo()
            .beginActivityWithOptions_reason(
                objc2_foundation::NSActivityOptions::UserInitiated,
                &reason,
            );
        std::mem::forget(activity);
    });
}

fn main() -> Result<()> {
    let renderer_name = std::env::var_os("TERMINATOR_RENDERER")
        .map(|name| {
            name.into_string()
                .map_err(|_| anyhow::anyhow!("TERMINATOR_RENDERER must be UTF-8"))
        })
        .transpose()?;
    let renderer = renderer::selected(renderer_name.as_deref())?;
    if !installation::preflight()? {
        return Ok(());
    }
    let args = std::env::args().collect::<Vec<_>>();
    let paths = if let Some(i) = args.iter().position(|a| a == "--data-dir") {
        Paths::at(
            args.get(i.checked_add(1).context("Missing data directory")?)
                .context("Missing data directory")?
                .into(),
        )
    } else {
        Paths::discover()?
    };
    let paths = generations::workspace_paths(&paths)?;
    paths.init()?;
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    hold_fixture_activity();
    crash::install(crash::CrashInstall {
        binary: "terminator",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: paths.data.clone(),
        notify: Some(installation::show_crash_dialog),
    });
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(paths.runtime.join("ui.lock"))?;
    if lock.try_lock_exclusive().is_err() {
        eprintln!(
            "Terminator is already open (lock {}).",
            paths.runtime.join("ui.lock").display()
        );
        return Ok(());
    }
    daemon_connection::ensure_running(&paths, &std::env::current_exe()?)?;
    let window_size = [1440.0, 900.0];
    #[cfg(feature = "test-support")]
    let window_size = {
        let scale = std::env::var("TERMINATOR_TEST_SCALE")
            .ok()
            .and_then(|s| s.parse::<f32>().ok())
            .filter(|s| (1.0..=2.0).contains(s))
            .unwrap_or(1.0);
        let size = if std::env::var_os("TERMINATOR_TEST_NARROW").is_some() {
            [900.0, 650.0]
        } else {
            window_size
        };
        [size[0] * scale, size[1] * scale]
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!(
                    "../assets/branding/terminator.png"
                ))
                .context("bundled Terminator icon must be a valid PNG")?,
            )
            .with_active(
                !(cfg!(feature = "test-support")
                    && std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
                    && std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some()
                    && std::env::var_os("TERMINATOR_TEST_FOREGROUND").is_none()),
            )
            .with_mouse_passthrough(
                cfg!(feature = "test-support")
                    && std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
                    && std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some()
                    && std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_none(),
            )
            .with_window_level(
                if cfg!(feature = "test-support")
                    && std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
                    && std::env::var_os("TERMINATOR_TEST_VISIBLE_CAPTURE").is_some()
                {
                    egui::WindowLevel::AlwaysOnTop
                } else if cfg!(feature = "test-support")
                    && std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
                    && std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some()
                {
                    egui::WindowLevel::AlwaysOnBottom
                } else {
                    egui::WindowLevel::Normal
                },
            )
            .with_inner_size(window_size)
            .with_min_inner_size([900.0, 550.0])
            .with_fullsize_content_view(cfg!(target_os = "macos"))
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true)
            .with_movable_by_background(false)
            .with_decorations(cfg!(target_os = "macos")),
        renderer,
        ..Default::default()
    };
    #[cfg(all(target_os = "macos", feature = "test-support"))]
    let options = {
        let mut options = options;
        if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some()
            && std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some()
        {
            options.event_loop_builder = Some(Box::new(|builder| {
                use egui_winit::winit::platform::macos::{
                    ActivationPolicy, EventLoopBuilderExtMacOS,
                };
                builder
                    .with_activation_policy(ActivationPolicy::Accessory)
                    .with_activate_ignoring_other_apps(false)
                    .with_default_menu(false);
            }));
        }
        options
    };
    #[cfg(all(feature = "test-support", feature = "wgpu"))]
    let options = {
        let mut options = options;
        let handler = std::sync::Arc::clone(&options.wgpu_options.on_surface_status);
        let count = std::sync::atomic::AtomicU32::new(0);
        options.wgpu_options.on_surface_status = std::sync::Arc::new(move |status| {
            if count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 8 {
                eprintln!("Native Wgpu surface status: {status:?}");
            }
            handler(status)
        });
        options
    };
    #[cfg(feature = "test-support")]
    eprintln!("Native renderer: {:?}", options.renderer);
    eframe::run_native(
        "Terminator",
        options,
        Box::new(move |cc| {
            #[cfg(all(feature = "test-support", feature = "wgpu"))]
            if let Some(state) = &cc.wgpu_render_state {
                eprintln!("Native Wgpu adapter: {:?}", state.adapter.get_info());
            }
            Ok(Box::new(App::new(cc, paths)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
