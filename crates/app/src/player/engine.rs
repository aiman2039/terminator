//! Playback worker. Decode and mix off the GUI thread.
use anyhow::{Context, Result, bail};
use rodio::Source as _;
use std::{
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
    Play(Playable),
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    Volume(f32),
    Shutdown,
}

enum Event {
    Status(Status),
    Finished,
}

pub struct Handle {
    commands: Sender<Command>,
    events: Receiver<Event>,
}

impl Handle {
    pub fn spawn() -> Self {
        let (commands, command_rx) = mpsc::channel();
        let (event_tx, events) = mpsc::channel();
        thread::Builder::new()
            .name("terminator-player".into())
            .spawn(move || Worker::run(command_rx, event_tx))
            .ok();
        Self { commands, events }
    }

    pub fn play(&self, source: Playable) {
        let _ = self.commands.send(Command::Play(source));
    }

    pub fn pause(&self) {
        let _ = self.commands.send(Command::Pause);
    }

    pub fn resume(&self) {
        let _ = self.commands.send(Command::Resume);
    }

    pub fn stop(&self) {
        let _ = self.commands.send(Command::Stop);
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
                Ok(Event::Status(status)) => out.push(Outcome::Status(status)),
                Ok(Event::Finished) => out.push(Outcome::Finished),
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
}

impl Worker {
    fn run(commands: Receiver<Command>, events: Sender<Event>) {
        let mut worker = Self {
            events,
            stream: None,
            sink: None,
            title: String::new(),
            duration: None,
            seekable: false,
            playing: false,
        };
        loop {
            match commands.recv_timeout(Duration::from_millis(50)) {
                Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                Ok(Command::Play(source)) => worker.play(source),
                Ok(Command::Pause) => worker.pause(),
                Ok(Command::Resume) => worker.resume(),
                Ok(Command::Stop) => worker.stop(),
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
        let sink = rodio::Sink::connect_new(stream.mixer());
        self.stream = Some(stream);
        self.sink = Some(sink);
        Ok(())
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
        let response = ureq::get(url)
            .timeout(Duration::from_secs(15))
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
        if let Some(sink) = &self.sink {
            sink.set_volume(volume.clamp(0.0, 1.0));
        }
    }

    fn tick(&mut self) {
        let Some(sink) = &self.sink else {
            return;
        };
        if self.playing && sink.empty() {
            self.playing = false;
            let _ = self.events.send(Event::Finished);
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
        let _ = self.events.send(Event::Status(status));
    }
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
    fn decoder_accepts_wav_bytes() {
        let wav = pcm_wav(&[0; 800], 8_000);
        assert!(rodio::Decoder::new(Cursor::new(wav)).is_ok());
    }

    #[test]
    fn decoder_rejects_empty_payload() {
        assert!(rodio::Decoder::new(Cursor::new(Vec::<u8>::new())).is_err());
    }
}
