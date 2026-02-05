use anyhow::Result;
use hound::WavWriter;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tracing::{error, info};
use uuid::Uuid;

use crate::config::AudioConfig;

pub struct AudioRecorder {
    config: AudioConfig,
    recording_path: PathBuf,
    running: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    writer: Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>,
    amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
}

impl AudioRecorder {
    pub fn new(
        config: &AudioConfig,
        amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
    ) -> Result<Self> {
        let recording_id = Uuid::new_v4().to_string();
        let recording_path =
            PathBuf::from(&config.temp_dir).join(format!("recording_{}.wav", recording_id));
        std::fs::create_dir_all(&config.temp_dir)?;

        Ok(Self {
            config: config.clone(),
            recording_path,
            running: Arc::new(AtomicBool::new(false)),
            thread: None,
            writer: Arc::new(Mutex::new(None)),
            amplitude_tx,
        })
    }

    pub async fn start(&mut self) -> Result<PathBuf> {
        info!(
            "Starting audio recording to: {}",
            self.recording_path.display()
        );

        let path = self.recording_path.clone();
        let sample_rate = self.config.sample_rate;
        let channels = self.config.channels as u32;

        // Create WAV writer (F32 at requested sample rate)
        let spec = hound::WavSpec {
            channels: channels as u16,
            sample_rate,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let wav_writer = WavWriter::create(&path, spec)?;
        *self.writer.lock().map_err(|_| anyhow::anyhow!("Writer mutex poisoned"))? = Some(wav_writer);

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let writer = self.writer.clone();
        let amp_tx = self.amplitude_tx.clone();

        let thread = std::thread::spawn(move || {
            if let Err(e) =
                run_pipewire_capture(running, writer, amp_tx, sample_rate, channels)
            {
                error!("Audio capture error: {}", e);
            }
        });

        self.thread = Some(thread);

        // Brief delay for PipeWire to connect
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        Ok(self.recording_path.clone())
    }

    pub async fn stop(&mut self) -> Result<PathBuf> {
        info!("Stopping audio recording");

        self.running.store(false, Ordering::SeqCst);

        if let Some(thread) = self.thread.take() {
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("Recording thread panicked"))?;
        }

        // Finalize WAV
        if let Ok(mut guard) = self.writer.lock() {
            if let Some(writer) = guard.take() {
                writer.finalize()?;
            }
        }

        let metadata = tokio::fs::metadata(&self.recording_path).await?;
        if metadata.len() < 100 {
            return Err(anyhow::anyhow!(
                "Recording file is too small ({} bytes)",
                metadata.len()
            ));
        }

        info!(
            "Recording saved: {} ({} bytes)",
            self.recording_path.display(),
            metadata.len()
        );

        Ok(self.recording_path.clone())
    }

    pub async fn cleanup(&self) -> Result<()> {
        if self.recording_path.exists() {
            tokio::fs::remove_file(&self.recording_path).await?;
        }
        Ok(())
    }
}

fn run_pipewire_capture(
    running: Arc<AtomicBool>,
    writer: Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>,
    amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
    sample_rate: u32,
    channels: u32,
) -> Result<()> {
    pipewire::init();

    let mainloop = pipewire::main_loop::MainLoop::new(None)
        .map_err(|e| anyhow::anyhow!("Failed to create PipeWire main loop: {:?}", e))?;
    let context = pipewire::context::Context::new(&mainloop)
        .map_err(|e| anyhow::anyhow!("Failed to create PipeWire context: {:?}", e))?;
    let core = context
        .connect(None)
        .map_err(|e| anyhow::anyhow!("Failed to connect to PipeWire: {:?}", e))?;

    let props = pipewire::properties::properties! {
        *pipewire::keys::MEDIA_TYPE => "Audio",
        *pipewire::keys::MEDIA_CATEGORY => "Capture",
        *pipewire::keys::MEDIA_ROLE => "Communication",
    };

    let stream = pipewire::stream::Stream::new(&core, "tjvox-capture", props)
        .map_err(|e| anyhow::anyhow!("Failed to create PipeWire stream: {:?}", e))?;

    // Build format parameters: F32LE at specified sample rate and channels
    let audio_params = build_audio_params(sample_rate, channels)?;
    let pod = pipewire::spa::pod::Pod::from_bytes(&audio_params)
        .ok_or_else(|| anyhow::anyhow!("Failed to create SPA pod from audio params"))?;

    // Window size for amplitude computation (50ms)
    let window_samples = (sample_rate as usize / 20) * channels as usize;

    struct CaptureState {
        writer: Arc<Mutex<Option<WavWriter<BufWriter<std::fs::File>>>>>,
        amp_buffer: Vec<f32>,
        amplitude_tx: Option<std::sync::mpsc::Sender<f32>>,
        window_samples: usize,
    }

    let state = CaptureState {
        writer,
        amp_buffer: Vec::with_capacity(window_samples * 2),
        amplitude_tx,
        window_samples,
    };

    // Get raw pointer for quitting from callback (safe: same thread)
    let raw_mainloop = mainloop.as_raw_ptr();
    let running_check = running.clone();

    let _listener = stream
        .add_local_listener_with_user_data(state)
        .process(move |stream, state| {
            if !running_check.load(Ordering::Relaxed) {
                unsafe { pipewire::sys::pw_main_loop_quit(raw_mainloop); }
                return;
            }

            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if let Some(d) = datas.first_mut() {
                    let chunk = d.chunk();
                    let size = chunk.size() as usize;
                    if size == 0 {
                        return;
                    }
                    if let Some(raw) = d.data() {
                        let audio_bytes = &raw[..size.min(raw.len())];
                        // Verify alignment before casting to f32 slice
                        if audio_bytes.as_ptr() as usize % std::mem::align_of::<f32>() != 0 {
                            return; // Skip unaligned buffer
                        }
                        let samples: &[f32] = unsafe {
                            std::slice::from_raw_parts(
                                audio_bytes.as_ptr() as *const f32,
                                audio_bytes.len() / std::mem::size_of::<f32>(),
                            )
                        };

                        // Write to WAV
                        if let Ok(mut guard) = state.writer.try_lock() {
                            if let Some(ref mut w) = *guard {
                                for &sample in samples {
                                    let _ = w.write_sample(sample);
                                }
                            }
                        }

                        // Compute amplitude (RMS per window)
                        state.amp_buffer.extend_from_slice(samples);
                        while state.amp_buffer.len() >= state.window_samples {
                            let window: Vec<f32> =
                                state.amp_buffer.drain(..state.window_samples).collect();
                            let sum_sq: f32 = window.iter().map(|s| s * s).sum();
                            let rms = (sum_sq / window.len() as f32).sqrt();
                            if let Some(ref tx) = state.amplitude_tx {
                                let _ = tx.send(rms);
                            }
                        }
                    }
                }
            }
        })
        .register()
        .map_err(|e| anyhow::anyhow!("Failed to register stream listener: {:?}", e))?;

    // Connect the stream
    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            None,
            pipewire::stream::StreamFlags::AUTOCONNECT
                | pipewire::stream::StreamFlags::MAP_BUFFERS
                | pipewire::stream::StreamFlags::RT_PROCESS,
            &mut [pod],
        )
        .map_err(|e| anyhow::anyhow!("Failed to connect PipeWire stream: {:?}", e))?;

    info!("PipeWire audio capture started");
    mainloop.run();
    info!("PipeWire audio capture stopped");

    Ok(())
}

fn build_audio_params(sample_rate: u32, channels: u32) -> Result<Vec<u8>> {
    use pipewire::spa::pod::serialize::PodSerializer;
    use pipewire::spa::pod::{Object, Property, PropertyFlags, Value};
    use pipewire::spa::sys;
    use pipewire::spa::utils::Id;

    let bytes = PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(Object {
            type_: pipewire::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: pipewire::spa::param::ParamType::EnumFormat.as_raw(),
            properties: vec![
                Property {
                    key: sys::SPA_FORMAT_mediaType,
                    flags: PropertyFlags::empty(),
                    value: Value::Id(Id(sys::SPA_MEDIA_TYPE_audio)),
                },
                Property {
                    key: sys::SPA_FORMAT_mediaSubtype,
                    flags: PropertyFlags::empty(),
                    value: Value::Id(Id(sys::SPA_MEDIA_SUBTYPE_raw)),
                },
                Property {
                    key: sys::SPA_FORMAT_AUDIO_format,
                    flags: PropertyFlags::empty(),
                    value: Value::Id(Id(sys::SPA_AUDIO_FORMAT_F32_LE)),
                },
                Property {
                    key: sys::SPA_FORMAT_AUDIO_rate,
                    flags: PropertyFlags::empty(),
                    value: Value::Int(sample_rate as i32),
                },
                Property {
                    key: sys::SPA_FORMAT_AUDIO_channels,
                    flags: PropertyFlags::empty(),
                    value: Value::Int(channels as i32),
                },
            ],
        }),
    )
    .map_err(|e| anyhow::anyhow!("Failed to serialize audio params: {:?}", e))?
    .0
    .into_inner();

    Ok(bytes)
}
