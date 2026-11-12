//! Retention: age, a per-file entry cap, and a total size budget.
use super::*;
use std::collections::{HashMap, HashSet};

/// What [`Store::prune`] keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Days kept; `0` keeps everything (recording is off).
    pub max_age_days: u32,
    /// Total size of the snapshots kept, in bytes. Oldest entries go first.
    pub total_bytes: u64,
    /// Newest snapshots kept per file.
    pub file_entries: usize,
}

impl Limits {
    /// The limits a settings model asks for.
    #[must_use]
    pub fn new(max_age_days: u32, total_mib: u64, file_entries: u32) -> Self {
        Self {
            max_age_days,
            total_bytes: total_mib.saturating_mul(1024 * 1024),
            file_entries: usize::try_from(file_entries).unwrap_or(usize::MAX),
        }
    }
}

/// One file's timeline while pruning.
struct Timeline {
    index: PathBuf,
    entries: Vec<Entry>,
    /// Lines in the file before pruning.
    before: usize,
}

impl Store {
    /// Drops entries and labels older than `max_age_days`, then all but the
    /// newest `file_entries` of each file, then the oldest entries of all
    /// files until the snapshots they keep fit in `total_bytes`, then the
    /// blobs no entry uses. `0` days keeps everything (recording is off).
    pub fn prune(&self, now_ms: u64, limits: Limits) -> Result<Pruned> {
        let mut pruned = Pruned::default();
        if limits.max_age_days == 0 || !self.root.exists() {
            return Ok(pruned);
        }
        let _lock = self.lock()?;
        let cutoff = now_ms.saturating_sub(DAY_MS.saturating_mul(u64::from(limits.max_age_days)));
        let mut timelines = Vec::new();
        for index in list_dir(&self.root.join("files"))? {
            let entries: Vec<Entry> = read_lines(&index)?;
            let before = entries.len();
            // Index lines are oldest first.
            let mut entries: Vec<Entry> = entries
                .into_iter()
                .filter(|e| e.timestamp_ms >= cutoff)
                .collect();
            let over = entries.len().saturating_sub(limits.file_entries);
            entries.drain(..over);
            timelines.push(Timeline {
                index,
                entries,
                before,
            });
        }
        fit_budget(&mut timelines, limits.total_bytes);
        let mut used = HashSet::new();
        for timeline in &timelines {
            let kept = timeline.entries.len();
            pruned.entries = pruned
                .entries
                .saturating_add(timeline.before.saturating_sub(kept));
            used.extend(timeline.entries.iter().map(|e| e.id.clone()));
            if kept == 0 {
                fs::remove_file(&timeline.index)?;
            } else if kept != timeline.before {
                write_lines(&timeline.index, &timeline.entries)?;
            }
        }
        self.prune_labels(cutoff)?;
        for blob in list_dir(&self.root.join("blobs"))? {
            let name = blob.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !used.contains(name) {
                fs::remove_file(&blob)?;
                pruned.blobs = pruned.blobs.saturating_add(1);
            }
        }
        Ok(pruned)
    }

    fn prune_labels(&self, cutoff: u64) -> Result<()> {
        let labels_path = self.root.join("labels.jsonl");
        let labels: Vec<LabelRecord> = read_lines(&labels_path)?;
        let total = labels.len();
        let labels: Vec<LabelRecord> = labels
            .into_iter()
            .filter(|l| l.timestamp_ms >= cutoff)
            .collect();
        if labels.len() != total {
            write_lines(&labels_path, &labels)?;
        }
        Ok(())
    }
}

/// Removes the oldest entries of all timelines until the distinct blobs
/// they use fit in `budget` bytes. A blob shared by several entries counts
/// once and is freed with its last user.
fn fit_budget(timelines: &mut [Timeline], budget: u64) {
    let mut blobs: HashMap<&str, (u64, usize)> = HashMap::new();
    for entry in timelines.iter().flat_map(|t| &t.entries) {
        let blob = blobs.entry(entry.id.as_str()).or_insert((entry.size, 0));
        blob.1 = blob.1.saturating_add(1);
    }
    let mut total: u64 = blobs.values().map(|(size, _)| *size).sum();
    if total <= budget {
        return;
    }
    let mut order: Vec<(u64, usize, usize)> = timelines
        .iter()
        .enumerate()
        .flat_map(|(t, timeline)| {
            timeline
                .entries
                .iter()
                .enumerate()
                .map(move |(e, entry)| (entry.timestamp_ms, t, e))
        })
        .collect();
    order.sort_unstable();
    let mut dropped: HashSet<(usize, usize)> = HashSet::new();
    for (_, t, e) in order {
        if total <= budget {
            break;
        }
        let Some(entry) = timelines
            .get(t)
            .and_then(|timeline| timeline.entries.get(e))
        else {
            continue;
        };
        if let Some(blob) = blobs.get_mut(entry.id.as_str()) {
            blob.1 = blob.1.saturating_sub(1);
            if blob.1 == 0 {
                total = total.saturating_sub(blob.0);
            }
        }
        dropped.insert((t, e));
    }
    drop(blobs);
    for (t, timeline) in timelines.iter_mut().enumerate() {
        let mut e = 0;
        timeline.entries.retain(|_| {
            let keep = !dropped.contains(&(t, e));
            e = e.saturating_add(1);
            keep
        });
    }
}
