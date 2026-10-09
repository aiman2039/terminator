use super::super::tap;
use super::analysis::download;
use super::decode::Permanent;
use super::handle::{COMPRESSED_SLOTS, Controls, Event, Handle, Outcome, Playable, Status};
use super::pipeline::PreparedSource;

use std::{
    num::NonZero,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(capacity: usize) -> (rtrb::Producer<f32>, PreparedSource) {
        let (pcm, consumer) = rtrb::RingBuffer::new(capacity);
        let (tap, _consumer) = rtrb::RingBuffer::new(tap::FFT_N);
        (
            pcm,
            PreparedSource {
                pcm: consumer,
                tap,
                controls: Arc::new(Controls {
                    generation: AtomicU64::new(1),
                    paused: AtomicBool::new(false),
                    volume: AtomicU32::new(1.0_f32.to_bits()),
                    decoders: AtomicUsize::new(0),
                    connections: AtomicUsize::new(0),
                }),
                counters: Arc::default(),
                generation: 1,
                channels: NonZero::new(1).unwrap(),
                rate: NonZero::new(8000).unwrap(),
                channel: 0,
                mix: 0.0,
            },
        )
    }
    #[test]
    fn pending_progress_cannot_undo_local_pause_or_resume() {
        let (handle, _commands, status, _) = Handle::channels();
        handle.play(Playable::File {
            path: "fixture.wav".into(),
            title: "Fixture".into(),
        });
        handle.pause();
        status.send_replace(Event {
            generation: 1,
            status: Status::Playing {
                title: "Fixture".into(),
                position: Duration::from_secs(1),
                duration: None,
                seekable: true,
            },
            finished: false,
        });
        assert!(matches!(
            handle.poll().as_slice(),
            [Outcome::Status(Status::Paused { .. })]
        ));
        handle.resume();
        status.send_replace(Event {
            generation: 1,
            status: Status::Paused {
                title: "Fixture".into(),
                position: Duration::from_secs(1),
                duration: None,
                seekable: true,
            },
            finished: false,
        });
        assert!(handle.poll().is_empty());
    }

    #[test]
    #[ignore = "real output-device local audio lifecycle check; muted"]
    fn local_audio_pause_seek_resume_and_natural_completion() {
        let directory = tempfile::Builder::new()
            .prefix("audio-local-")
            .tempdir_in("/tmp")
            .unwrap();
        let path = directory.path().join("silence.wav");
        let data_len = 8000_u32 * 3 * 2;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16_u32.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(1_u16.to_le_bytes());
        wav.extend(8000_u32.to_le_bytes());
        wav.extend(16000_u32.to_le_bytes());
        wav.extend(2_u16.to_le_bytes());
        wav.extend(16_u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        std::fs::write(&path, wav).unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let (service, mut owner) = crate::gui_services::Services::new(
            terminator_core::Paths::at(directory.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let handle = Handle::spawn(service);
        handle.volume(0.0);
        handle.play(Playable::File {
            path,
            title: "Local fixture".into(),
        });
        let mut wait = |predicate: &dyn Fn(&Outcome) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(6);
            loop {
                while owner.supervisor.try_recv().is_some() {}
                for outcome in handle.poll() {
                    if let Outcome::Status(Status::Error(error)) = &outcome {
                        panic!("Local audio fixture: {error}");
                    }
                    if predicate(&outcome) {
                        return;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "Local audio lifecycle did not progress"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait(
            &|event| matches!(event, Outcome::Status(Status::Playing { position, .. }) if *position >= Duration::from_millis(50)),
        );
        handle.pause();
        wait(&|event| matches!(event, Outcome::Status(Status::Paused { .. })));
        handle.seek(Duration::from_secs(1));
        wait(
            &|event| matches!(event, Outcome::Status(Status::Paused { position, .. }) if *position == Duration::from_secs(1)),
        );
        handle.resume();
        wait(
            &|event| matches!(event, Outcome::Status(Status::Playing { position, .. }) if *position > Duration::from_secs(1)),
        );
        wait(&|event| matches!(event, Outcome::Finished));
        assert_eq!(handle.controls.decoders.load(Ordering::Acquire), 0);
    }

    #[test]
    #[ignore = "bounded live 103FM network and real output-device check; muted"]
    fn live_103fm_reaches_pcm_and_stop_releases_pipeline() {
        let directory = tempfile::Builder::new()
            .prefix("radio-live-")
            .tempdir_in("/tmp")
            .unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let (service, mut owner) = crate::gui_services::Services::new(
            terminator_core::Paths::at(directory.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let handle = Handle::spawn(service);
        handle.volume(0.0);
        let start = Instant::now();
        handle.play(Playable::Stream {
            title: "103FM live fixture".into(),
            url: "https://cdn.cybercdn.live/103FM/Live/icecast.audio".into(),
        });
        let mut first_pcm = None;
        while start.elapsed() < Duration::from_secs(20) {
            while owner.supervisor.try_recv().is_some() {}
            for outcome in handle.poll() {
                match outcome {
                    Outcome::Status(Status::Playing { position, .. })
                        if position >= Duration::from_millis(250) =>
                    {
                        first_pcm = Some(start.elapsed());
                    }
                    Outcome::Status(Status::Error(error)) => {
                        handle.stop();
                        panic!("103FM live fixture: {error}");
                    }
                    _ => {}
                }
            }
            if first_pcm.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let stop = Instant::now();
        handle.stop();
        while handle.controls.decoders.load(Ordering::Acquire) != 0
            || handle.controls.connections.load(Ordering::Acquire) != 0
        {
            assert!(
                stop.elapsed() < Duration::from_secs(2),
                "Stopped live pipeline remained active"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            first_pcm.is_some(),
            "103FM did not produce PCM within 20 seconds"
        );
        println!(
            "{}",
            serde_json::json!({"station":"103FM","pcm_ms":first_pcm.unwrap().as_secs_f64()*1000.0,"cleanup_ms":stop.elapsed().as_secs_f64()*1000.0,"output_device":true,"muted":true})
        );
    }

    #[test]
    fn callback_never_waits_and_stop_invalidates_already_buffered_pcm() {
        let (mut producer, mut callback) = prepared(2);
        let started = Instant::now();
        for _ in 0..10000 {
            assert_eq!(callback.next(), Some(0.0));
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        producer.push(0.75).unwrap();
        producer.push(0.5).unwrap();
        assert!(producer.push(1.0).is_err());
        callback.controls.generation.store(2, Ordering::Release);
        assert_eq!(callback.next(), None);
    }
    #[test]
    fn local_pause_keeps_samples_and_saved_mute_applies_before_first_sample() {
        let (mut producer, mut callback) = prepared(4);
        producer.push(0.5).unwrap();
        callback.controls.paused.store(true, Ordering::Release);
        assert_eq!(callback.next(), Some(0.0));
        callback.controls.paused.store(false, Ordering::Release);
        callback
            .controls
            .volume
            .store(0.0_f32.to_bits(), Ordering::Release);
        assert_eq!(callback.next(), Some(0.0));
        assert_eq!(callback.counters.consumed.load(Ordering::Acquire), 1);
        callback.counters.finished.store(true, Ordering::Release);
        assert_eq!(callback.next(), None);
        assert!(callback.counters.drained.load(Ordering::Acquire));
    }
    #[test]
    fn rapid_switching_keeps_only_latest_request_and_old_finish_cannot_advance_it() {
        let handle = Handle::finished_fixture();
        for n in 0..100 {
            handle.play(Playable::Stream {
                url: format!("http://127.0.0.1/{n}"),
                title: n.to_string(),
            });
        }
        assert_eq!(handle.desired.borrow().generation, 100);
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 100);
        assert!(handle.poll().is_empty());
        handle.pause();
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 101);
        handle.resume();
        assert_eq!(handle.controls.generation.load(Ordering::Acquire), 102);
        handle.stop();
        assert!(handle.desired.borrow().source.is_none());
    }
    async fn server(
        header_delay: Duration,
        gaps: Vec<Duration>,
        header: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).await;
            tokio::time::sleep(header_delay).await;
            if socket.write_all(header).await.is_err() {
                return;
            }
            for gap in gaps {
                tokio::time::sleep(gap).await;
                if socket.write_all(b"x").await.is_err() {
                    break;
                }
            }
        });
        (format!("http://{address}"), task)
    }
    #[tokio::test]
    async fn slow_headers_and_stalled_reads_have_separate_deadlines() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let headers = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n";
        for (block_headers, expected) in [(true, "header"), (false, "stalled")] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (release, blocked) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                if !block_headers && socket.write_all(headers).await.is_err() {
                    return;
                }
                // Keep the selected phase blocked until the client reports
                // its deadline. A delayed executor cannot make data race it.
                let _ = blocked.await;
                if block_headers {
                    let _ = socket.write_all(headers).await;
                }
                let _ = socket.write_all(b"x").await;
            });
            let (tx, _rx) = mpsc::channel(COMPRESSED_SLOTS);
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                download(
                    &client,
                    format!("http://{address}"),
                    tx,
                    if block_headers {
                        Duration::from_millis(100)
                    } else {
                        Duration::from_secs(5)
                    },
                    Duration::from_millis(100),
                ),
            )
            .await;
            let _ = release.send(());
            server.abort();
            if let Err(error) = server.await {
                assert!(error.is_cancelled(), "Fixture server failed: {error}");
            }
            let error = result
                .expect("Client did not enforce its deadline")
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error:#}");
        }
    }
    #[tokio::test]
    async fn fragmented_healthy_radio_has_no_total_lifetime_deadline() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (url, server) = server(
            Duration::ZERO,
            vec![Duration::from_millis(750); 4],
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n",
        )
        .await;
        let (tx, mut rx) = mpsc::channel(COMPRESSED_SLOTS);
        let read_deadline = Duration::from_secs(2);
        let started = Instant::now();
        download(&client, url, tx, Duration::from_secs(5), read_deadline)
            .await
            .unwrap();
        assert!(
            started.elapsed() > read_deadline,
            "Fixture must outlive a single read deadline"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = rx.recv().await {
            bytes.extend(chunk);
        }
        assert_eq!(bytes, b"xxxx");
        server.await.unwrap();
    }
    #[tokio::test]
    async fn permanent_http_error_is_not_a_retryable_station_failure() {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let (url, server) = server(
            Duration::ZERO,
            vec![],
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let (tx, _rx) = mpsc::channel(COMPRESSED_SLOTS);
        assert!(
            download(
                &client,
                url,
                tx,
                Duration::from_secs(1),
                Duration::from_secs(1)
            )
            .await
            .unwrap_err()
            .downcast_ref::<Permanent>()
            .is_some()
        );
        server.await.unwrap();
    }
}
