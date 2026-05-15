use cpal::traits::{DeviceTrait, HostTrait};
use rodio::{OutputStream, OutputStreamHandle, Sink};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::oneshot;

/// Audio player using a dedicated worker thread so rodio's non-Send stream
/// types stay thread-confined while the public API remains async + Send-safe.
pub struct AudioPlayer {
    preferred_output_device: Option<String>,
    follow_system_default: bool,
    worker: Arc<Mutex<Option<PlayerWorker>>>,
}

struct PlaybackRuntime {
    _stream: OutputStream,
    handle: OutputStreamHandle,
    sink: Sink,
    healthy: bool,
}

struct PlayerWorker {
    tx: std::sync::mpsc::Sender<PlayerCommand>,
}

enum PlayerCommand {
    PlayFile {
        path: PathBuf,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    PlaySamples {
        samples: Vec<f32>,
        sample_rate: u32,
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Stop {
        reply: oneshot::Sender<anyhow::Result<()>>,
    },
    Invalidate,
    Health {
        reply: oneshot::Sender<bool>,
    },
}

impl AudioPlayer {
    pub fn new() -> Self {
        Self {
            preferred_output_device: None,
            follow_system_default: true,
            worker: Arc::new(Mutex::new(None)),
        }
    }

    /// Prefer a specific output device by name.
    /// Use None or "auto" to use system default.
    pub fn with_output_device(mut self, device_name: Option<String>) -> Self {
        self.preferred_output_device = device_name.and_then(|raw| {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        self
    }

    /// Whether playback should always follow the current system default speaker.
    pub fn follow_system_default(mut self, follow: bool) -> Self {
        self.follow_system_default = follow;
        self
    }

    /// Play WAV file.
    pub async fn play_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        let worker = self.ensure_worker()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        worker
            .tx
            .send(PlayerCommand::PlayFile {
                path: path.to_path_buf(),
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("audio playback worker unavailable"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("audio playback worker dropped response"))?
    }

    /// Play raw PCM f32 samples.
    pub async fn play_samples(&self, samples: Vec<f32>, sample_rate: u32) -> anyhow::Result<()> {
        let worker = self.ensure_worker()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        worker
            .tx
            .send(PlayerCommand::PlaySamples {
                samples,
                sample_rate,
                reply: reply_tx,
            })
            .map_err(|_| anyhow::anyhow!("audio playback worker unavailable"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("audio playback worker dropped response"))?
    }

    /// Stop playback immediately and re-create the sink while keeping the
    /// underlying output stream open.
    pub async fn stop_now(&self) -> anyhow::Result<()> {
        let worker = self.ensure_worker()?;
        let (reply_tx, reply_rx) = oneshot::channel();
        worker
            .tx
            .send(PlayerCommand::Stop { reply: reply_tx })
            .map_err(|_| anyhow::anyhow!("audio playback worker unavailable"))?;
        reply_rx
            .await
            .map_err(|_| anyhow::anyhow!("audio playback worker dropped response"))?
    }

    /// Mark current runtime unhealthy; next playback lazily reopens it.
    pub fn invalidate_runtime(&self) {
        if let Ok(worker) = self.ensure_worker() {
            let _ = worker.tx.send(PlayerCommand::Invalidate);
        }
    }

    pub fn is_healthy(&self) -> bool {
        let Ok(worker) = self.ensure_worker() else {
            return false;
        };
        let (reply_tx, mut reply_rx) = oneshot::channel();
        if worker
            .tx
            .send(PlayerCommand::Health { reply: reply_tx })
            .is_err()
        {
            return false;
        }
        // Use try_recv to avoid blocking the runtime if called from async context.
        // If we can't get an immediate response, assume unhealthy to avoid stalling.
        reply_rx.try_recv().unwrap_or(false)
    }

    fn ensure_worker(&self) -> anyhow::Result<PlayerWorker> {
        let mut guard = self
            .worker
            .lock()
            .map_err(|_| anyhow::anyhow!("audio worker lock poisoned"))?;
        if let Some(existing) = guard.as_ref() {
            return Ok(PlayerWorker {
                tx: existing.tx.clone(),
            });
        }

        let preferred = self.preferred_output_device.clone();
        let follow_default = self.follow_system_default;
        let (tx, rx) = std::sync::mpsc::channel::<PlayerCommand>();
        std::thread::Builder::new()
            .name("kria-audio-playback".to_string())
            .spawn(move || run_playback_worker(rx, preferred, follow_default))
            .map_err(|e| anyhow::anyhow!("failed to spawn playback worker: {e}"))?;
        let worker = PlayerWorker { tx: tx.clone() };
        *guard = Some(PlayerWorker { tx });
        Ok(worker)
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new()
    }
}

/// Enumerate available output device names.
pub fn list_output_devices() -> anyhow::Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();

    if let Ok(devices) = host.output_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                names.push(name);
            }
        }
    }

    names.sort();
    names.dedup();
    Ok(names)
}

/// Return current system default output device name.
pub fn default_output_device_name() -> Option<String> {
    let host = cpal::default_host();
    host.default_output_device().and_then(|d| d.name().ok())
}

fn run_playback_worker(
    rx: std::sync::mpsc::Receiver<PlayerCommand>,
    preferred: Option<String>,
    follow_default: bool,
) {
    let mut runtime: Option<PlaybackRuntime> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            PlayerCommand::PlayFile { path, reply } => {
                let res = (|| {
                    let rt = ensure_runtime(&mut runtime, preferred.as_deref(), follow_default)?;
                    let file = std::io::BufReader::new(std::fs::File::open(&path)?);
                    let source = rodio::Decoder::new(file)?;
                    rt.sink.append(source);
                    rt.sink.sleep_until_end();
                    Ok::<_, anyhow::Error>(())
                })();
                if res.is_err() {
                    runtime = None;
                }
                let _ = reply.send(res);
            }
            PlayerCommand::PlaySamples {
                samples,
                sample_rate,
                reply,
            } => {
                let res = (|| {
                    let rt = ensure_runtime(&mut runtime, preferred.as_deref(), follow_default)?;
                    let source = rodio::buffer::SamplesBuffer::new(1, sample_rate, samples);
                    rt.sink.append(source);
                    rt.sink.sleep_until_end();
                    Ok::<_, anyhow::Error>(())
                })();
                if res.is_err() {
                    runtime = None;
                }
                let _ = reply.send(res);
            }
            PlayerCommand::Stop { reply } => {
                let res = (|| {
                    if let Some(rt) = runtime.as_mut() {
                        rt.sink.stop();
                        rt.sink = Sink::try_new(&rt.handle)?;
                        rt.healthy = true;
                    }
                    Ok::<_, anyhow::Error>(())
                })();
                let _ = reply.send(res);
            }
            PlayerCommand::Invalidate => {
                runtime = None;
            }
            PlayerCommand::Health { reply } => {
                let healthy = runtime.as_ref().map(|rt| rt.healthy).unwrap_or(false);
                let _ = reply.send(healthy);
            }
        }
    }
}

fn open_output_stream(
    preferred_device: Option<&str>,
    follow_system_default: bool,
) -> anyhow::Result<(OutputStream, OutputStreamHandle)> {
    if !follow_system_default {
        if let Some(requested) = preferred_device {
            let requested = requested.trim();
            if !requested.is_empty() && !requested.eq_ignore_ascii_case("auto") {
                let host = cpal::default_host();
                if let Ok(devices) = host.output_devices() {
                    for device in devices {
                        if device.name().ok().as_deref() == Some(requested) {
                            tracing::info!(device = %requested, "audio playback using requested output device");
                            return OutputStream::try_from_device(&device).map_err(|e| {
                                anyhow::anyhow!("failed to open output device '{requested}': {e}")
                            });
                        }
                    }
                }
                tracing::warn!(
                    device = %requested,
                    "requested speaker not found, falling back to system default"
                );
            }
        }
    }

    OutputStream::try_default()
        .map_err(|e| anyhow::anyhow!("failed to open default output device: {e}"))
}

fn ensure_runtime<'a>(
    runtime: &'a mut Option<PlaybackRuntime>,
    preferred: Option<&str>,
    follow_default: bool,
) -> anyhow::Result<&'a mut PlaybackRuntime> {
    if runtime.is_none() {
        let (stream, handle) = open_output_stream(preferred, follow_default)?;
        let sink = Sink::try_new(&handle)?;
        *runtime = Some(PlaybackRuntime {
            _stream: stream,
            handle,
            sink,
            healthy: true,
        });
    }
    runtime
        .as_mut()
        .ok_or_else(|| anyhow::anyhow!("failed to initialize playback runtime"))
}
