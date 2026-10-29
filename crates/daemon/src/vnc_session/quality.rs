use terminator_core::vnc::{Quality, VncOptions};
use vnc::VncEncoding;
pub(super) fn encodings(options: &VncOptions) -> Vec<VncEncoding> {
    let mut list = match options.quality {
        Quality::Sharp => vec![VncEncoding::Zrle, VncEncoding::CopyRect, VncEncoding::Raw],
        Quality::LowBandwidth => vec![
            VncEncoding::Tight,
            VncEncoding::QualityLow,
            VncEncoding::CompressHigh,
            VncEncoding::Zrle,
            VncEncoding::CopyRect,
            VncEncoding::Raw,
        ],
        Quality::Auto => vec![
            VncEncoding::Tight,
            VncEncoding::Zrle,
            VncEncoding::CopyRect,
            VncEncoding::Raw,
        ],
    };
    // The cursor shape is always requested: the server then leaves the
    // pointer out of the framebuffer and the viewer draws it locally at the
    // local mouse position, so it never lags behind the frame stream.
    list.extend([
        VncEncoding::DesktopSizePseudo,
        VncEncoding::ExtendedDesktopSizePseudo,
        VncEncoding::CursorPseudo,
    ]);
    list
}
/// Shortest time between two update requests. Requests are pipelined, so
/// this caps the frame rate (about 60 per second) rather than adding delay.
/// `recent_bytes` is the last legacy encode; shared-memory frames leave it 0.
pub(super) fn refresh_ms(quality: Quality, recent_bytes: usize) -> u64 {
    match quality {
        Quality::LowBandwidth => 100,
        Quality::Sharp => 16,
        Quality::Auto => {
            if recent_bytes > 2 * 1024 * 1024 {
                66
            } else {
                16
            }
        }
    }
}
