use super::super::tap;
use super::decode::run;
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use tokio::sync::watch;
pub(super) const CHUNK: usize = 16 * 1024;
pub(super) const COMPRESSED_SLOTS: usize = 14; // + one producer and one reader chunk <= 256 KiB
pub(super) const MAX_PCM_SAMPLES: usize = 4 * 1024 * 1024 / std::mem::size_of::<f32>();
#[derive(Clone, Debug)]
pub(crate) enum Playable {
    File { path: PathBuf, title: String },
    Stream { url: String, title: String },
}
impl Playable {
    pub(super) fn title(&self) -> &str {
        match self {
            Self::File { title, .. } | Self::Stream { title, .. } => title,
        }
    }
    pub(super) fn radio(&self) -> bool {
        matches!(self, Self::Stream { .. })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Stopped,
    Loading {
        title: String,
    },
    Buffering {
        title: String,
    },
    Reconnecting {
        title: String,
        attempt: u8,
    },
    Playing {
        title: String,
        position: Duration,
        duration: Option<Duration>,
        seekable: bool,
    },
    Paused {
        title: String,
        position: Duration,
        duration: Option<Duration>,
        seekable: bool,
    },
    Error(String),
}
#[derive(Clone)]
pub(super) struct Desired {
    pub(super) generation: u64,
    pub(super) source: Option<Playable>,
    pub(super) offset: Duration,
}
#[derive(Clone, PartialEq)]
pub(super) struct Event {
    pub(super) generation: u64,
    pub(super) status: Status,
    pub(super) finished: bool,
}
pub(super) struct Controls {
    pub(super) generation: AtomicU64,
    pub(super) paused: AtomicBool,
    pub(super) volume: AtomicU32,
    pub(super) decoders: AtomicUsize,
    pub(super) connections: AtomicUsize,
}
pub(crate) struct Handle {
    pub(super) desired: watch::Sender<Desired>,
    pub(super) controls: Arc<Controls>,
    pub(super) events: RefCell<watch::Receiver<Event>>,
    pub(super) spectrum: watch::Receiver<Option<[f32; tap::BARS]>>,
    pub(super) lifetime: CancellationToken,
    pub(super) seen: RefCell<Option<Event>>,
}
pub(crate) enum Outcome {
    Status(Status),
    Finished,
}
type Channels = (
    Handle,
    watch::Receiver<Desired>,
    watch::Sender<Event>,
    watch::Sender<Option<[f32; tap::BARS]>>,
);
impl Handle {
    pub fn spawn(service: crate::gui_services::Services) -> Self {
        let (handle, commands, status, spectrum) = Self::channels();
        let controls = Arc::clone(&handle.controls);
        let lifetime = handle.lifetime.clone();
        let mut context = OperationContext::new("audio", "player".into(), Policy::ServiceLifetime);
        context.deadline = None;
        let services = service.clone();
        let errors = status.clone();
        let error_controls = Arc::clone(&controls);
        if let Err(error) = service
            .handle()
            .submit(context, lifetime.clone(), async move {
                if let Err(error) = run(
                    services,
                    commands,
                    controls,
                    status.clone(),
                    spectrum,
                    lifetime,
                )
                .await
                {
                    let generation = error_controls.generation.load(Ordering::Acquire);
                    status.send_replace(Event {
                        generation,
                        status: Status::Error(format!("{error:#}")),
                        finished: false,
                    });
                }
                Ok(Vec::new())
            })
        {
            errors.send_replace(Event {
                generation: 0,
                status: Status::Error(error.to_string()),
                finished: false,
            });
        }
        handle
    }
    pub(super) fn channels() -> Channels {
        let (desired, commands) = watch::channel(Desired {
            generation: 0,
            source: None,
            offset: Duration::ZERO,
        });
        let (status, events) = watch::channel(Event {
            generation: 0,
            status: Status::Stopped,
            finished: false,
        });
        let (spectrum_tx, spectrum) = watch::channel(None);
        (
            Self {
                desired,
                controls: Arc::new(Controls {
                    generation: AtomicU64::new(0),
                    paused: AtomicBool::new(false),
                    volume: AtomicU32::new(1.0_f32.to_bits()),
                    decoders: AtomicUsize::new(0),
                    connections: AtomicUsize::new(0),
                }),
                events: RefCell::new(events),
                spectrum,
                lifetime: CancellationToken::new(),
                seen: RefCell::new(None),
            },
            commands,
            status,
            spectrum_tx,
        )
    }
    #[cfg(test)]
    pub(crate) fn finished_fixture() -> Self {
        let (handle, _, status, _) = Self::channels();
        status.send_replace(Event {
            generation: 0,
            status: Status::Stopped,
            finished: true,
        });
        handle
    }
    #[cfg(feature = "test-support")]
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"generation":self.controls.generation.load(Ordering::Acquire), "decoders":self.controls.decoders.load(Ordering::Acquire), "connections":self.controls.connections.load(Ordering::Acquire)})
    }
    pub fn play(&self, source: Playable) {
        self.controls.paused.store(false, Ordering::Release);
        let generation = self
            .controls
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.desired.send_replace(Desired {
            generation,
            source: Some(source),
            offset: Duration::ZERO,
        });
    }
    pub fn stop(&self) {
        let generation = self
            .controls
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        self.desired.send_replace(Desired {
            generation,
            source: None,
            offset: Duration::ZERO,
        });
    }
    pub fn pause(&self) {
        self.controls.paused.store(true, Ordering::Release);
        self.desired.send_modify(|desired| {
            if desired.source.as_ref().is_some_and(Playable::radio) {
                desired.generation = self
                    .controls
                    .generation
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
            }
        });
    }
    pub fn resume(&self) {
        self.controls.paused.store(false, Ordering::Release);
        self.desired.send_modify(|desired| {
            if desired.source.as_ref().is_some_and(Playable::radio) {
                desired.generation = self
                    .controls
                    .generation
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
            }
        });
    }
    pub fn seek(&self, position: Duration) {
        self.desired.send_modify(|desired| {
            if matches!(desired.source, Some(Playable::File { .. })) {
                desired.offset = position;
                desired.generation = self
                    .controls
                    .generation
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
            }
        });
    }
    pub fn volume(&self, volume: f32) {
        self.controls
            .volume
            .store(volume.clamp(0.0, 1.0).to_bits(), Ordering::Release);
    }
    pub fn spectrum_bars(&self) -> Option<[f32; tap::BARS]> {
        if self.events.borrow().borrow().generation
            != self.controls.generation.load(Ordering::Acquire)
        {
            return None;
        }
        *self.spectrum.borrow()
    }
    pub fn poll(&self) -> Vec<Outcome> {
        let mut receiver = self.events.borrow_mut();
        // A closed watch still retains its last unseen value (including errors).
        if !receiver.has_changed().unwrap_or(true) {
            return Vec::new();
        }
        let mut event = receiver.borrow_and_update().clone();
        let paused = self.controls.paused.load(Ordering::Acquire);
        if paused {
            match event.status {
                Status::Playing {
                    title,
                    position,
                    duration,
                    seekable,
                } => {
                    event.status = Status::Paused {
                        title,
                        position,
                        duration,
                        seekable,
                    }
                }
                Status::Loading { .. } | Status::Buffering { .. } | Status::Reconnecting { .. } => {
                    return Vec::new();
                }
                _ => {}
            }
        } else if matches!(event.status, Status::Paused { .. }) {
            return Vec::new();
        }
        if event.generation != self.controls.generation.load(Ordering::Acquire)
            || self.seen.borrow().as_ref() == Some(&event)
        {
            return Vec::new();
        }
        self.seen.replace(Some(event.clone()));
        let mut result = vec![Outcome::Status(event.status)];
        if event.finished {
            result.push(Outcome::Finished);
        }
        result
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.stop();
        self.lifetime.cancel();
    }
}
