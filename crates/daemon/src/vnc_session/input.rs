use anyhow::{Result, ensure};
use terminator_core::vnc::{MAX_CLIPBOARD, VncInputEvent, VncOptions};
use vnc::{ClientKeyEvent, ClientMouseEvent, X11Event};
pub(super) fn validate(
    events: &[VncInputEvent],
    options: &VncOptions,
    view_only: bool,
) -> Result<()> {
    ensure!(events.len() <= 256, "VNC input batch exceeds 256 events");
    for event in events {
        match event {
            VncInputEvent::CopyText(text) => {
                ensure!(text.len() <= MAX_CLIPBOARD, "VNC clipboard exceeds 256 KiB");
                ensure!(
                    options.clipboard.send(),
                    "Sending the VNC clipboard is disabled"
                );
                ensure!(
                    text.chars().all(|c| u32::from(c) <= 255),
                    "This server uses a Latin-1 clipboard; use Paste as keystrokes for Unicode text"
                );
            }
            VncInputEvent::TypeText(text) => {
                ensure!(
                    text.chars().count() <= 4096,
                    "Paste as keystrokes is limited to 4096 characters"
                );
                ensure!(options.clipboard.send(), "Sending text to VNC is disabled");
            }
            VncInputEvent::Resize { width, height } => {
                ensure!(
                    !view_only && options.remote_resize,
                    "Remote resize is disabled"
                );
                ensure!(
                    *width > 0
                        && *height > 0
                        && *width <= 8192
                        && *height <= 8192
                        && usize::from(*width).saturating_mul(usize::from(*height))
                            <= 32 * 1024 * 1024,
                    "Remote desktop exceeds the supported dimensions"
                );
            }
            _ => {}
        }
    }
    Ok(())
}
pub(super) fn keysym(ch: char) -> Option<u32> {
    match ch {
        '\n' | '\r' => Some(0xFF0D),
        '\t' => Some(0xFF09),
        c if c.is_control() => None,
        c => {
            let c = u32::from(c);
            Some(if c <= 255 { c } else { 0x0100_0000 | c })
        }
    }
}
pub(super) fn to_x11_event(event: VncInputEvent) -> Option<X11Event> {
    Some(match event {
        VncInputEvent::Pointer { x, y, buttons } => X11Event::PointerEvent(ClientMouseEvent {
            position_x: x,
            position_y: y,
            bottons: buttons,
        }),
        VncInputEvent::Key { keycode, down } => {
            X11Event::KeyEvent(ClientKeyEvent { keycode, down })
        }
        VncInputEvent::CopyText(text) => X11Event::CopyText(text),
        VncInputEvent::Refresh { full } => {
            if full {
                X11Event::FullRefresh
            } else {
                X11Event::Refresh
            }
        }
        VncInputEvent::Resize { .. } | VncInputEvent::TypeText(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::vnc::ClipboardMode;
    #[test]
    fn clipboard_limits_and_direction_are_enforced() {
        let mut options = VncOptions::default();
        assert!(validate(&[VncInputEvent::CopyText("café".into())], &options, false).is_ok());
        assert!(validate(&[VncInputEvent::CopyText("漢".into())], &options, false).is_err());
        assert!(
            validate(
                &[VncInputEvent::CopyText("x".repeat(MAX_CLIPBOARD + 1))],
                &options,
                false
            )
            .is_err()
        );
        options.clipboard = ClipboardMode::ReceiveOnly;
        assert!(validate(&[VncInputEvent::CopyText("text".into())], &options, false).is_err());
        assert!(validate(&[VncInputEvent::TypeText("text".into())], &options, false).is_err());
    }
    #[test]
    fn unicode_keysyms_and_typed_text_are_bounded() {
        assert_eq!(keysym('漢'), Some(0x01006f22));
        assert_eq!(keysym('é'), Some(233));
        assert_eq!(keysym('\n'), Some(0xff0d));
        assert!(
            validate(
                &[VncInputEvent::TypeText("x".repeat(4097))],
                &VncOptions::default(),
                false
            )
            .is_err()
        );
    }
    #[test]
    fn resize_requires_permission_and_safe_dimensions() {
        let mut options = VncOptions::default();
        let resize = VncInputEvent::Resize {
            width: 800,
            height: 600,
        };
        assert!(validate(std::slice::from_ref(&resize), &options, false).is_err());
        options.remote_resize = true;
        assert!(validate(std::slice::from_ref(&resize), &options, false).is_ok());
        assert!(validate(&[resize], &options, true).is_err());
        assert!(
            validate(
                &[VncInputEvent::Resize {
                    width: 8192,
                    height: 8192
                }],
                &options,
                false
            )
            .is_err()
        );
    }
}
