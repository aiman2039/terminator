//! Right-sidebar Info tool: the focused session and host resources.
#[cfg(test)]
mod tests;
mod view;

pub use view::show;

use crate::resource_sample::SystemStats;
use terminator_sys::PressureLevel;

pub const SESSION: &str = "info-session";
pub const CWD: &str = "info-cwd";
pub const BRANCH: &str = "info-branch";
pub const STARTED: &str = "info-started";
pub const SESSION_CPU: &str = "info-session-cpu";
pub const SESSION_MEMORY: &str = "info-session-memory";
pub const SYSTEM_CPU: &str = "info-system-cpu";
pub const SYSTEM_MEMORY: &str = "info-system-memory";
pub const PRESSURE: &str = "info-pressure";
pub const LOAD: &str = "info-load";
pub const SYSTEM_TOGGLE: &str = "info-system-toggle";

const DASH: &str = "—";

#[derive(Clone, Debug)]
pub struct Input {
    pub label: Option<String>,
    pub cwd: Option<String>,
    pub cwd_full: Option<String>,
    pub branch: Option<String>,
    pub started: String,
    pub unconfirmed: bool,
    pub session_cpu: Option<f32>,
    pub session_memory: Option<u64>,
    pub system: Option<SystemStats>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub session: String,
    pub cwd: String,
    pub cwd_full: Option<String>,
    pub branch: Option<String>,
    pub started: String,
    pub unconfirmed: bool,
    pub session_cpu: String,
    pub session_memory: String,
    pub system: Option<SystemRows>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SystemRows {
    pub cpu: String,
    pub cpu_fraction: f32,
    pub memory: String,
    pub memory_fraction: f32,
    pub pressure: String,
    pub pressure_fraction: f32,
    pub pressure_level: Option<PressureLevel>,
    pub load: String,
    pub load_fraction: f32,
}

pub struct Toggles {
    pub process_open: bool,
    pub resources_open: bool,
    pub show_system: bool,
}

#[must_use]
pub fn started_label(created: u64, now: u64, live: bool) -> String {
    if !live || created == 0 {
        return DASH.into();
    }
    let age = now.saturating_sub(created);
    if age < 90 {
        format!("{age}s ago")
    } else if age < 5_400 {
        format!("{}m ago", age / 60)
    } else if age < 86_400 * 2 {
        format!("{}h ago", age / 3_600)
    } else {
        format!("{}d ago", age / 86_400)
    }
}

#[must_use]
pub fn format_percent(value: f32) -> String {
    if !value.is_finite() || value < 0.0 {
        return DASH.into();
    }
    if value >= 10.0 || value.fract() < 0.05 {
        format!("{value:.0}%")
    } else {
        format!("{value:.1}%")
    }
}

fn f32_from_f64(value: f64) -> f32 {
    let bits = value.to_bits();
    let sign = (bits >> 63) & 1;
    let exp_bits = (bits >> 52) & 0x7ff;
    let mant = bits & 0x000f_ffff_ffff_ffff;
    let Ok(exp_u) = u16::try_from(exp_bits) else {
        return 0.0;
    };
    if exp_u == 0x7ff {
        if mant != 0 {
            return f32::NAN;
        }
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if exp_u == 0 {
        return if sign == 0 { 0.0 } else { -0.0 };
    }
    let Some(unbiased) = i32::from(exp_u).checked_sub(1023) else {
        return 0.0;
    };
    if unbiased > 127 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    if unbiased < -126 {
        return if sign == 0 { 0.0 } else { -0.0 };
    }
    let mut mantissa = mant >> 29;
    let remainder = mant & ((1_u64 << 29) - 1);
    if remainder > (1_u64 << 28) || (remainder == (1_u64 << 28) && mantissa & 1 == 1) {
        mantissa = mantissa.saturating_add(1);
    }
    let mut exp32 = unbiased.saturating_add(127);
    if mantissa >= (1_u64 << 23) {
        mantissa = 0;
        exp32 = exp32.saturating_add(1);
    }
    if exp32 >= 255 {
        return if sign == 0 {
            f32::INFINITY
        } else {
            f32::NEG_INFINITY
        };
    }
    let Ok(exp32) = u32::try_from(exp32) else {
        return 0.0;
    };
    let Ok(mantissa) = u32::try_from(mantissa) else {
        return 0.0;
    };
    let sign = u32::try_from(sign).unwrap_or(0);
    f32::from_bits((sign << 31) | (exp32 << 23) | mantissa)
}

#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    if bytes == 0 {
        return "0 MB".into();
    }
    // Display size. Counts above 2^53 do not fit in an f64 mantissa.
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.0} MB", value / MB)
    } else if value >= KB {
        format!("{:.0} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

#[must_use]
pub fn format_load(one: f64, five: f64, fifteen: f64) -> String {
    format!("{one:.2} {five:.2} {fifteen:.2}")
}

#[must_use]
pub fn model(input: &Input) -> Model {
    let system = input.system.as_ref().map(|system| {
        let pressure_fraction = system
            .pressure
            .map(|pressure| (pressure.percent / 100.0).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        let pressure = system
            .pressure
            .map(|pressure| {
                format!(
                    "{} · {}",
                    format_percent(pressure.percent),
                    pressure.level.label()
                )
            })
            .unwrap_or_else(|| DASH.into());
        let cores = f64::from(u32::try_from(system.cpus.max(1)).unwrap_or(u32::MAX));
        SystemRows {
            cpu: format_percent(system.cpu),
            cpu_fraction: (system.cpu / 100.0).clamp(0.0, 1.0),
            memory: format!(
                "{} / {}",
                format_bytes(system.memory_used),
                format_bytes(system.memory_total)
            ),
            memory_fraction: if system.memory_total == 0 {
                0.0
            } else {
                {
                    // Memory meter. Byte counts can exceed the f32 mantissa.
                    let used = system.memory_used as f32;
                    let total = system.memory_total as f32;
                    (used / total).clamp(0.0, 1.0)
                }
            },
            pressure,
            pressure_fraction,
            pressure_level: system.pressure.map(|pressure| pressure.level),
            load: format_load(system.load_one, system.load_five, system.load_fifteen),
            load_fraction: f32_from_f64((system.load_one / cores).clamp(0.0, 1.0)),
        }
    });
    Model {
        session: input.label.clone().unwrap_or_else(|| DASH.into()),
        cwd: input.cwd.clone().unwrap_or_else(|| DASH.into()),
        cwd_full: input.cwd_full.clone(),
        branch: input.branch.clone().filter(|branch| !branch.is_empty()),
        started: input.started.clone(),
        unconfirmed: input.unconfirmed,
        session_cpu: input
            .session_cpu
            .map(format_percent)
            .unwrap_or_else(|| DASH.into()),
        session_memory: input
            .session_memory
            .map(format_bytes)
            .unwrap_or_else(|| DASH.into()),
        system,
    }
}
