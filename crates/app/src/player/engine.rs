//! Playback worker. Decode and mix off the GUI thread.
use anyhow::{Context, Result, bail};
use rodio::Source as _;
use std::{
    cell::Cell,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
    },
    thread,
    time::Duration,
};

#[derive(Clone, Debug)]
pub enum Playable {
    File { path: PathBuf, title: String },
    Stream { url: String, title: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Stopped,
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

enum Command {
    Play(u64, Playable),
    Pause,
    Resume,
    Stop(u64),
    Seek(Duration),
    Volume(f32),
    Shutdown,
}

struct Event {
    generation: u64,
    outcome: Outcome,
}

pub struct Handle {
    commands: Sender<Command>,
    events: Receiver<Event>,
    generation: Cell<u64>,
}

impl Handle {
    #[cfg(test)]
    pub(super) fn finished_fixture() -> Self {
        let (commands, _command_rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        tx.send(Event {
            generation: 0,
            outcome: Outcome::Finished,
        })
        .unwrap();
        tx.send(Event {
            generation: 0,
            outcome: Outcome::Status(Status::Stopped),
        })
        .unwrap();
        Self {
            commands,
            events,
            generation: Cell::new(0),
        }
    }

    pub fn spawn() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("terminator-player".into())
            .spawn(move || Worker::run(command_rx, event_tx))
            .ok();
        Self {
            commands,
            events,
            generation: Cell::new(0),
        }
    }

    pub fn play(&self, source: Playable) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let _ = self.commands.send(Command::Play(generation, source));
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn stop(&self) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let _ = self.commands.send(Command::Stop(generation));
    }

    pub fn seek(&self, position: Duration) {
        let _ = self.commands.send(Command::Seek(position));
    }

    pub fn volume(&self, volume: f32) {
        let _ = self.commands.send(Command::Volume(volume));
    }

    pub fn poll(&self) -> Vec<Outcome> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) if event.generation == self.generation.get() => out.push(event.outcome),
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.push(Outcome::Status(Status::Error(
                        "Audio worker stopped".into(),
                    )));
                    break;
                }
            }
        }
        out
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
    }
}

pub enum Outcome {
    Status(Status),
    Finished,
}

struct Worker {
    events: Sender<Event>,
    stream: Option<rodio::OutputStream>,
    sink: Option<rodio::Sink>,
    title: String,
    duration: Option<Duration>,
    seekable: bool,
    playing: bool,
    volume: f32,
    generation: u64,
}

impl Worker {
    fn new(events: Sender<Event>) -> Self {
        Self {
            events,
            stream: None,
            sink: None,
            title: String::new(),
            duration: None,
            seekable: false,
            playing: false,
            volume: 1.0,
            generation: 0,
        }
    }

    fn run(commands: Receiver<Command>, events: Sender<Event>) {
        let mut worker = Self::new(events);
        loop {
            match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Command::Play(generation, source)) => {
                    worker.generation = generation;
                    worker.play(source);
                }
                Ok(Command::Pause) => worker.pause(),
                Ok(Command::Resume) => worker.resume(),
                Ok(Command::Stop(generation)) => {
                    worker.generation = generation;
                    worker.stop();
                }
                Ok(Command::Seek(position)) => worker.seek(position),
                Ok(Command::Volume(volume)) => worker.set_volume(volume),
                Err(RecvTimeoutError::Timeout) => worker.tick(),
            }
        }
    }

    fn ensure_output(&mut self) -> Result<()> {
        if self.sink.is_some() {
            return Ok(());
        }
        let stream =
            rodio::OutputStreamBuilder::open_default_stream().context("No audio output device")?;
        let sink = self.new_sink(stream.mixer());
        self.stream = Some(stream);
        self.sink = Some(sink);
        Ok(())
    }

    fn new_sink(&self, mixer: &rodio::mixer::Mixer) -> rodio::Sink {
        let sink = rodio::Sink::connect_new(mixer);
        sink.set_volume(self.volume);
        sink
    }

    fn play(&mut self, source: Playable) {
        if let Err(error) = self.play_source(source) {
            self.playing = false;
            self.emit(Status::Error(format!("{error:#}")));
        }
    }

    fn play_source(&mut self, source: Playable) -> Result<()> {
        self.ensure_output()?;
        match source {
            Playable::File { path, title } => self.append_file(&path, title)?,
            Playable::Stream { url, title } => self.append_stream(&url, title)?,
        }
        let sink = self.sink.as_ref().context("Audio output missing")?;
        sink.play();
        self.playing = true;
        self.emit(self.playing_status());
        Ok(())
    }

    fn append_file(&mut self, path: &std::path::Path, title: String) -> Result<()> {
        let sink = self.sink.as_ref().context("Audio output missing")?;
        sink.clear();
        let file = File::open(path).with_context(|| format!("Open {}", path.display()))?;
        let decoder = rodio::Decoder::try_from(file).context("Decode audio")?;
        self.duration = decoder.total_duration();
        self.seekable = true;
        self.title = title;
        sink.append(decoder);
        Ok(())
    }

    fn append_stream(&mut self, url: &str, title: String) -> Result<()> {
        let sink = self.sink.as_ref().context("Audio output missing")?;
        sink.clear();
        let response = stream_agent(Duration::from_secs(15))
            .get(url)
            .call()
            .with_context(|| format!("Connect {url}"))?;
        let content_type = response.content_type().to_ascii_lowercase();
        if content_type.contains("html") || content_type.contains("json") {
            bail!("Stream is not audio ({content_type})");
        }
        let decoder = rodio::Decoder::builder()
            .with_data(StreamReader::from_read(response.into_reader()))
            .with_seekable(false)
            .build()
            .context("Decode stream")?;
        self.duration = decoder.total_duration();
        self.seekable = false;
        self.title = title;
        sink.append(decoder);
        Ok(())
    }

    fn pause(&mut self) {
        if let Some(sink) = &self.sink {
            sink.pause();
            self.playing = false;
            self.emit(Status::Paused {
                title: self.title.clone(),
                position: sink.get_pos(),
                duration: self.duration,
                seekable: self.seekable,
            });
        }
    }

    fn resume(&mut self) {
        if let Some(sink) = &self.sink {
            sink.play();
            self.playing = true;
            self.emit(self.playing_status());
        }
    }

    fn stop(&mut self) {
        if let Some(sink) = &self.sink {
            sink.clear();
            sink.pause();
        }
        self.playing = false;
        self.title.clear();
        self.duration = None;
        self.seekable = false;
        self.emit(Status::Stopped);
    }

    fn seek(&mut self, position: Duration) {
        let Some(sink) = &self.sink else {
            return;
        };
        if !self.seekable {
            return;
        }
        if sink.try_seek(position).is_err() {
            self.emit(Status::Error("Seek failed".into()));
            return;
        }
        if self.playing {
            self.emit(self.playing_status());
        } else {
            self.pause();
        }
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = &self.sink {
            sink.set_volume(self.volume);
        }
    }

    fn tick(&mut self) {
        let Some(sink) = &self.sink else {
            return;
        };
        if self.playing && sink.empty() {
            self.playing = false;
            let _ = self.events.send(Event {
                generation: self.generation,
                outcome: Outcome::Finished,
            });
            self.emit(Status::Stopped);
            return;
        }
        if self.playing {
            self.emit(self.playing_status());
        }
    }

    fn playing_status(&self) -> Status {
        let position = self
            .sink
            .as_ref()
            .map_or(Duration::ZERO, rodio::Sink::get_pos);
        Status::Playing {
            title: self.title.clone(),
            position,
            duration: self.duration,
            seekable: self.seekable,
        }
    }

    fn emit(&self, status: Status) {
        let _ = self.events.send(Event {
            generation: self.generation,
            outcome: Outcome::Status(status),
        });
    }
}

// A live response has no total deadline. Each stalled connection/read is bounded.
fn stream_agent(timeout: Duration) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeout)
        .timeout_read(timeout)
        .timeout_write(timeout)
        .build()
}

struct StreamInner {
    reader: Box<dyn Read + Send>,
    pos: u64,
}

struct StreamReader {
    inner: Mutex<StreamInner>,
}

impl StreamReader {
    fn from_read(reader: impl Read + Send + 'static) -> Self {
        Self {
            inner: Mutex::new(StreamInner {
                reader: Box::new(reader),
                pos: 0,
            }),
        }
    }
}

impl Read for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("audio stream lock"))?;
        let n = inner.reader.read(buf)?;
        inner.pos += n as u64;
        Ok(n)
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let inner = self
            .inner
            .lock()
            .map_err(|_| io::Error::other("audio stream lock"))?;
        match from {
            SeekFrom::Current(0) => Ok(inner.pos),
            SeekFrom::Start(0) if inner.pos == 0 => Ok(0),
            SeekFrom::Start(offset) if offset == inner.pos => Ok(inner.pos),
            _ => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "live stream is not seekable",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    fn pcm_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
        let data_len = u32::try_from(samples.len() * 2).unwrap_or(0);
        let mut out = Vec::with_capacity(44 + samples.len() * 2);
        out.extend(b"RIFF");
        out.extend(&(36 + data_len).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(&16u32.to_le_bytes());
        out.extend(&1u16.to_le_bytes());
        out.extend(&1u16.to_le_bytes());
        out.extend(&sample_rate.to_le_bytes());
        out.extend(&(sample_rate * 2).to_le_bytes());
        out.extend(&2u16.to_le_bytes());
        out.extend(&16u16.to_le_bytes());
        out.extend(b"data");
        out.extend(&data_len.to_le_bytes());
        for sample in samples {
            out.extend(&sample.to_le_bytes());
        }
        out
    }

    #[test]
    fn completion_from_an_old_source_cannot_advance_the_new_source() {
        let handle = super::Handle::finished_fixture();
        handle.play(super::Playable::Stream {
            url: "https://example.com/radio".into(),
            title: "Radio".into(),
        });
        assert!(
            !handle
                .poll()
                .iter()
                .any(|event| matches!(event, super::Outcome::Finished))
        );
    }

    #[test]
    fn saved_mute_is_applied_before_the_first_source() {
        let (tx, _) = std::sync::mpsc::channel();
        let mut worker = super::Worker::new(tx);
        worker.set_volume(0.0);
        let (mixer, _source) = rodio::mixer::mixer(1, 8000);
        let sink = worker.new_sink(&mixer);
        assert_eq!(sink.volume(), 0.0);
    }

    #[test]
    fn streaming_reader_has_no_total_lifetime_deadline() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Duration,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 1024];
            assert!(socket.read(&mut request).unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                .unwrap();
            for _ in 0..4 {
                thread::sleep(Duration::from_millis(400));
                socket.write_all(b"x").unwrap();
            }
        });
        let response = super::stream_agent(Duration::from_secs(1))
            .get(&format!("http://{address}"))
            .call()
            .unwrap();
        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        assert_eq!(body, "xxxx");
        server.join().unwrap();
    }

    #[test]
    fn decoder_accepts_wav_bytes() {
        let wav = pcm_wav(&[0; 800], 8_000);
        assert!(rodio::Decoder::new(Cursor::new(wav)).is_ok());
    }

    #[test]
    fn decoder_rejects_empty_payload() {
        assert!(rodio::Decoder::new(Cursor::new(Vec::<u8>::new())).is_err());
    }
}
