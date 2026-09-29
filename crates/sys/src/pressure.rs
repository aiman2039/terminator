//! Host memory pressure.
//!
//! Linux reads PSI `some avg10` from `/proc/pressure/memory`. macOS reads the
//! kernel level (`kern.memorystatus_vm_pressure_level`) and the compressor's
//! share of physical memory. The level word is the kernel's own
//! normal/warning/urgent/critical state. The percent is a 0–100 bar.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MemoryPressure {
    pub percent: f32,
    pub level: PressureLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PressureLevel {
    Normal,
    Warning,
    Urgent,
    Critical,
}

impl PressureLevel {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Urgent => "urgent",
            Self::Critical => "critical",
        }
    }
}

#[must_use]
#[cfg(target_os = "linux")]
pub fn memory_pressure() -> Option<MemoryPressure> {
    linux()
}

#[must_use]
#[cfg(target_os = "macos")]
pub fn memory_pressure() -> Option<MemoryPressure> {
    macos()
}

#[must_use]
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn memory_pressure() -> Option<MemoryPressure> {
    None
}

/// PSI `some avg10` is the percent of time at least one task stalled on memory.
#[cfg(any(test, target_os = "linux"))]
fn pressure_from_psi(text: &str) -> Option<MemoryPressure> {
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("some ") else {
            continue;
        };
        for token in rest.split_whitespace() {
            let Some(value) = token.strip_prefix("avg10=") else {
                continue;
            };
            let percent = value.parse::<f32>().ok()?;
            if !percent.is_finite() {
                return None;
            }
            let percent = percent.clamp(0.0, 100.0);
            return Some(MemoryPressure {
                percent,
                level: level_from_percent(percent),
            });
        }
    }
    None
}

#[cfg(any(test, target_os = "linux"))]
fn level_from_percent(percent: f32) -> PressureLevel {
    if percent < 25.0 {
        PressureLevel::Normal
    } else if percent < 70.0 {
        PressureLevel::Warning
    } else {
        PressureLevel::Critical
    }
}

#[cfg(target_os = "linux")]
fn linux() -> Option<MemoryPressure> {
    let text = std::fs::read_to_string("/proc/pressure/memory").ok()?;
    pressure_from_psi(&text)
}

#[cfg(any(test, target_os = "macos"))]
fn level_from_code(code: i32) -> PressureLevel {
    match code {
        2 => PressureLevel::Warning,
        4 => PressureLevel::Urgent,
        8 => PressureLevel::Critical,
        _ => PressureLevel::Normal,
    }
}

/// Compressor pages over physical memory. Stays low while file cache is
/// reclaimable, which is the gap between "memory used" and "pressure".
#[cfg(any(test, target_os = "macos"))]
fn compressor_percent(compressor_pages: u64, page_size: u64, total_bytes: u64) -> Option<f32> {
    if page_size == 0 || total_bytes == 0 {
        return None;
    }
    let bytes = compressor_pages.saturating_mul(page_size);
    Some((bytes as f32 / total_bytes as f32 * 100.0).clamp(0.0, 100.0))
}

#[cfg(target_os = "macos")]
fn macos() -> Option<MemoryPressure> {
    let level = read_level();
    let percent = read_compressor_percent();
    if level.is_none() && percent.is_none() {
        return None;
    }
    let percent = percent.unwrap_or(0.0);
    Some(MemoryPressure {
        percent,
        level: level.unwrap_or(PressureLevel::Normal),
    })
}

#[cfg(target_os = "macos")]
fn read_level() -> Option<PressureLevel> {
    let mut level: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>();
    let rc = unsafe {
        libc::sysctlbyname(
            c"kern.memorystatus_vm_pressure_level".as_ptr(),
            &mut level as *mut libc::c_int as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then(|| level_from_code(level))
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // libc::mach_host_self; mach2 is not a workspace dependency.
fn read_compressor_percent() -> Option<f32> {
    let page = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
    if page == 0 {
        return None;
    }
    let mut total: u64 = 0;
    let mut total_len = std::mem::size_of::<u64>();
    let total_rc = unsafe {
        libc::sysctlbyname(
            c"hw.memsize".as_ptr(),
            &mut total as *mut u64 as *mut libc::c_void,
            &mut total_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if total_rc != 0 || total == 0 {
        return None;
    }
    let port = HostPort(unsafe { libc::mach_host_self() });
    if port.0 == 0 {
        return None;
    }
    let mut count = libc::HOST_VM_INFO64_COUNT;
    let mut stat = unsafe { std::mem::zeroed::<libc::vm_statistics64>() };
    let rc = unsafe {
        libc::host_statistics64(
            port.0,
            libc::HOST_VM_INFO64,
            &mut stat as *mut libc::vm_statistics64 as *mut _,
            &mut count,
        )
    };
    if rc != libc::KERN_SUCCESS {
        return None;
    }
    // `vm_statistics64` is packed. Copy the field instead of borrowing it.
    let pages = unsafe { std::ptr::addr_of!(stat.compressor_page_count).read_unaligned() };
    compressor_percent(u64::from(pages), page, total)
}

#[cfg(target_os = "macos")]
struct HostPort(libc::mach_port_t);

#[cfg(target_os = "macos")]
impl Drop for HostPort {
    #[allow(deprecated)] // libc::mach_task_self; mach2 is not a workspace dependency.
    fn drop(&mut self) {
        if self.0 != 0 {
            unsafe {
                mach_port_deallocate(libc::mach_task_self(), self.0);
            }
        }
    }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    fn mach_port_deallocate(
        task: libc::mach_port_t,
        name: libc::mach_port_t,
    ) -> libc::kern_return_t;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn psi_some_avg10_is_memory_pressure() {
        let text = "\
some avg10=10.00 avg60=2.00 avg300=1.00 total=9
full avg10=80.00 avg60=0.00 avg300=0.00 total=0
";
        let pressure = pressure_from_psi(text).unwrap();
        assert!((pressure.percent - 10.0).abs() < f32::EPSILON);
        assert_eq!(pressure.level, PressureLevel::Normal);
        assert_eq!(pressure.level.label(), "normal");
    }

    #[test]
    fn psi_thresholds_leave_normal_above_10_percent() {
        assert_eq!(
            pressure_from_psi("some avg10=25.00 avg60=0 total=0\n")
                .unwrap()
                .level,
            PressureLevel::Warning
        );
        assert_eq!(
            pressure_from_psi("some avg10=70.00 avg60=0 total=0\n")
                .unwrap()
                .level,
            PressureLevel::Critical
        );
        assert!(pressure_from_psi("full avg10=10.00 avg60=0 total=0\n").is_none());
    }

    #[test]
    fn pressure_level_codes_match_the_kernel() {
        assert_eq!(level_from_code(0), PressureLevel::Normal);
        assert_eq!(level_from_code(1), PressureLevel::Normal);
        assert_eq!(level_from_code(2), PressureLevel::Warning);
        assert_eq!(level_from_code(4), PressureLevel::Urgent);
        assert_eq!(level_from_code(8), PressureLevel::Critical);
    }

    #[test]
    fn compressor_percent_is_pages_over_physical_memory() {
        let page = 4096u64;
        let percent = compressor_percent(10, page, 100 * page).unwrap();
        assert!((percent - 10.0).abs() < 0.01);
        assert!(compressor_percent(1, 0, 100).is_none());
        assert!(compressor_percent(1, page, 0).is_none());
    }
}
