//! Why settings or a state transition were rejected.

/// Why a hook event or terminal notice could not be applied to the state.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("Unknown live session")]
    UnknownLiveSession { session: String },
    #[error("Unsupported hook version")]
    UnsupportedHookVersion { version: u32 },
    #[error("Unknown or ended session")]
    UnknownOrEndedSession { session: String },
    #[error("Missing event identity")]
    MissingEventIdentity { session: String },
    #[error("Hook too large")]
    HookTooLarge { event: String },
}

/// Why user settings are invalid.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("ntfy channel must contain 1–128 letters, digits, underscores or hyphens when enabled")]
    NtfyChannel { channel: String },
    #[error("ntfy machine name must be at most 128 bytes without control characters")]
    NtfyMachine { machine: String },
    #[error("History age must be 1–3650 days")]
    HistoryDays { days: u64 },
    #[error("Invalid disk limits")]
    DiskLimits { session_mib: u64, total_mib: u64 },
    #[error("History lines must be 100–20000")]
    ScrollbackLines { lines: usize },
    #[error("Font size must be 9–32")]
    FontSize { size: f32 },
    #[error("Editor close timeout must be 1–30 seconds")]
    EditorCloseTimeout { seconds: u64 },
}
