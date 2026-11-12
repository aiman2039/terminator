//! Local wall-clock time through `localtime_r` and `mktime`.

/// Seconds from the Unix time `now` until `hour`:00 local time on the next
/// day. `None` when the C library cannot convert the time.
#[must_use]
pub fn seconds_until_tomorrow_at(now: u64, hour: i32) -> Option<u64> {
    let stamp = libc::time_t::try_from(now).ok()?;
    // SAFETY: `libc::tm` is plain C data; all-zero is a valid value.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid for the call; `localtime_r` is the
    // thread-safe form and writes only into `tm`.
    let converted = unsafe { libc::localtime_r(&raw const stamp, &raw mut tm) };
    if converted.is_null() {
        return None;
    }
    tm.tm_mday = tm.tm_mday.saturating_add(1);
    tm.tm_hour = hour;
    tm.tm_min = 0;
    tm.tm_sec = 0;
    tm.tm_isdst = -1;
    // SAFETY: `tm` is a valid, initialised struct that `mktime` normalises.
    let target = unsafe { libc::mktime(&raw mut tm) };
    u64::try_from(target).ok()?.checked_sub(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tomorrow_morning_is_later_today_or_tomorrow() {
        let now = 1_750_000_000;
        let secs = seconds_until_tomorrow_at(now, 9).unwrap();
        assert!((60..=34 * 60 * 60).contains(&secs), "{secs}");
    }
}
