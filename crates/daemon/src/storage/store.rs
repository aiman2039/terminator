use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, params};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
use terminator_core::*;

use super::decode;

pub struct Store {
    pub(super) conn: Connection,
    owner: Option<String>,
}
impl Store {
    pub fn open(paths: &Paths) -> Result<(Self, State)> {
        // Probe the version first: a newer database must fail loudly and
        // must never be clobbered by the quarantine path below. An
        // unreadable probe (missing or garbage file) falls through to a
        // fresh or quarantined open.
        if let Ok(conn) = Connection::open(paths.data.join("state.sqlite3")) {
            let version: u32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap_or(0);
            ensure!(version <= 1, "Database is newer than this application");
        }
        match Self::read(paths) {
            Ok(opened) => Ok(opened),
            // Disk full, read-only or busy is not damage: never quarantine a
            // good store for it.
            Err(error) if !decode::corrupt(&error) => Err(error),
            Err(error) => {
                // A damaged store must never prevent startup: quarantine the
                // files for forensics and start from defaults. If the
                // directory itself is unusable the fresh open fails too and
                // that error propagates, as before.
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                for suffix in ["", "-wal", "-shm", "-journal"] {
                    let from = paths.data.join(format!("state.sqlite3{suffix}"));
                    if from.exists() {
                        let _ = fs::rename(
                            &from,
                            paths
                                .data
                                .join(format!("state.corrupt-{stamp}.sqlite3{suffix}")),
                        );
                    }
                }
                let (store, mut state) = Self::read(paths).context(
                    "Saved state was unreadable and quarantine did not produce a fresh store",
                )?;
                state.degraded = Some(format!(
                    "Saved state was unreadable and was quarantined; started with defaults: {error:#}"
                ));
                Ok((store, state))
            }
        }
    }
    fn read(paths: &Paths) -> Result<(Self, State)> {
        let conn = Connection::open(paths.data.join("state.sqlite3"))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=3000;",
        )?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS app_state (id INTEGER PRIMARY KEY CHECK(id=1), json TEXT NOT NULL); PRAGMA user_version=1;")?;
        let mut state: State =
            match conn.query_row::<String, _, _>("SELECT json FROM app_state WHERE id=1", [], |r| {
                r.get(0)
            }) {
                Ok(json) => decode::state(&json)?,
                Err(rusqlite::Error::QueryReturnedNoRows) => State::default(),
                Err(e) => return Err(e.into()),
            };
        if state.settings.validate().is_err() {
            // Persisted settings that fail current validation could never
            // have been saved by this build (saves are validated), so they
            // come from corruption or hand edits. Reset rather than running
            // with values no code path accepts.
            state.settings = Settings::default();
            state.degraded =
                Some("Stored settings failed validation and were reset to defaults.".into());
        }
        // Presence is live-only; nothing persisted may claim it.
        state.presence.clear();
        state.terminal_titles.clear();
        Ok((Self { conn, owner: None }, state))
    }
    pub fn bind_owner(&mut self, owner: String) {
        self.owner = Some(owner);
    }
    pub fn save(&self, state: &State) -> Result<()> {
        let mut owned;
        let mut stripped;
        let state = if let Some(owner) = &self.owner {
            ensure!(&state.generation == owner, "Session store owner changed");
            ensure!(
                state.sessions.iter().all(|s| &s.generation == owner),
                "Foreign session in owner store"
            );
            owned = state.clone();
            owned.projects.clear();
            owned.init_commands.clear();
            owned.worktrees.clear();
            owned.vnc_profiles.clear();
            owned.selected_project = None;
            owned.generations.clear();
            owned.presence.clear();
            &owned
        } else if state.presence.is_empty() {
            state
        } else {
            // Presence is live-only and never persisted; no migration needed.
            stripped = state.clone();
            stripped.presence.clear();
            &stripped
        };
        self.conn.execute("INSERT INTO app_state(id,json) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET json=excluded.json WHERE json_extract(excluded.json,'$.revision') >= json_extract(app_state.json,'$.revision')",params![serde_json::to_string(state)?])?;
        Ok(())
    }
}
