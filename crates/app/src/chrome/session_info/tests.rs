use super::*;
use crate::resource_sample::SystemStats;
use terminator_sys::{MemoryPressure, PressureLevel};

fn system() -> SystemStats {
    SystemStats {
        cpu: 29.0,
        memory_used: 47_137_266_073,
        memory_total: 128 * 1024 * 1024 * 1024,
        pressure: Some(MemoryPressure {
            percent: 10.0,
            level: PressureLevel::Normal,
        }),
        load_one: 6.70,
        load_five: 5.43,
        load_fifteen: 6.15,
        cpus: 10,
    }
}

#[test]
fn info_model_formats_process_and_system_rows() {
    let model = model(&Input {
        label: Some("Terminal 3".into()),
        cwd: Some("~/RustroverProjects/terminator".into()),
        cwd_full: Some("/Users/ohaddahan/RustroverProjects/terminator".into()),
        branch: Some("master".into()),
        started: "2m ago".into(),
        unconfirmed: false,
        session_cpu: None,
        session_memory: None,
        system: Some(system()),
    });
    assert_eq!(model.session, "Terminal 3");
    assert_eq!(model.cwd, "~/RustroverProjects/terminator");
    assert_eq!(model.branch.as_deref(), Some("master"));
    assert_eq!(model.started, "2m ago");
    assert_eq!(model.session_cpu, DASH);
    assert_eq!(model.session_memory, DASH);
    let system = model.system.unwrap();
    assert_eq!(system.cpu, "29%");
    assert!((system.cpu_fraction - 0.29).abs() < 0.001);
    assert!(system.memory.starts_with("43."));
    assert!(system.memory.contains("128.0 GB"));
    assert!((system.memory_fraction - 43.9 / 128.0).abs() < 0.02);
    assert_eq!(system.pressure, "10% · normal");
    assert!((system.pressure_fraction - 0.10).abs() < 0.001);
    assert_eq!(system.load, "6.70 5.43 6.15");
    assert!((system.load_fraction - 0.67).abs() < 0.001);
}

#[test]
fn info_started_label_is_blank_until_the_process_is_live() {
    assert_eq!(started_label(1_000, 1_120, false), DASH);
    assert_eq!(started_label(0, 1_120, true), DASH);
    assert_eq!(started_label(1_000, 1_120, true), "2m ago");
    assert_eq!(started_label(1_000, 1_000 + 3_600 * 3, true), "3h ago");
    assert_eq!(started_label(1_000, 1_000 + 86_400 * 3, true), "3d ago");
}

#[test]
fn info_bytes_use_one_decimal_for_gigabytes() {
    assert_eq!(format_bytes(0), "0 MB");
    assert_eq!(format_bytes(44 * 1024 * 1024 * 1024), "44.0 GB");
    assert_eq!(format_bytes(12 * 1024 * 1024), "12 MB");
}
