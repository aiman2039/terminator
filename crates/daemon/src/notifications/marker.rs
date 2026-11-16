//! The invisible per-waiter marker, and removing the banner that carries it.

use super::*;

static NONCE: OnceLock<u64> = OnceLock::new();

/// A unique, invisible suffix for one waiter. Zero-width joiners, spaces, and
/// non-joiners encode a per-process nonce mixed with the generation. The mix
/// (`SplitMix64`) makes markers of consecutive generations differ in about
/// half their bits, so a partly stripped marker cannot pass for a neighbour's.
pub(super) fn marker_for(generation: u64) -> String {
    marker_from_key(mix(process_nonce().wrapping_add(generation)))
}

/// `SplitMix64` finalizer.
fn mix(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub(super) fn marker_from_key(key: u64) -> String {
    let mut marker = String::with_capacity(65);
    marker.push('\u{200D}');
    for bit in (0..64).rev() {
        marker.push(if (key >> bit) & 1 == 1 {
            '\u{200C}'
        } else {
            '\u{200B}'
        });
    }
    marker
}

fn process_nonce() -> u64 {
    *NONCE.get_or_init(|| {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX))
            .unwrap_or(0);
        (u64::from(std::process::id())).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ now
    })
}

/// Remove only `generation`'s notification from Notification Center. Its
/// disappearance makes the `ObjC` dismiss poll resolve an auto-dismiss, releasing
/// the waiter so the worker can present the alert that superseded it. Returns
/// whether this daemon's own banner was found and removed; banners of other
/// daemons, generations or orphans are never touched.
#[cfg(target_os = "macos")]
#[expect(
    deprecated,
    reason = "NSUserNotificationCenter is the only API for removing delivered notifications here"
)]
pub(super) fn remove_active_notification(generation: u64, summary: &str) -> bool {
    if generation == 0 {
        return false;
    }
    let marker = marker_for(generation);
    use objc2_foundation::NSUserNotificationCenter;
    let center = NSUserNotificationCenter::defaultUserNotificationCenter();
    let delivered = center.deliveredNotifications();
    let snapshots = delivered
        .iter()
        .map(|notification| {
            (
                notification
                    .title()
                    .map(|title| title.to_string())
                    .unwrap_or_default(),
                notification
                    .informativeText()
                    .map(|text| text.to_string())
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let banners = snapshots
        .iter()
        .map(|(title, text)| Banner { title, text })
        .collect::<Vec<_>>();
    let indexes = banners_to_remove(&banners, &marker, summary);
    if indexes.is_empty() {
        return false;
    }
    for (index, notification) in delivered.iter().enumerate() {
        if indexes.contains(&index) {
            center.removeDeliveredNotification(&notification);
        }
    }
    true
}
