//! Audio capture: WASAPI loopback (system sound) + microphone via cpal,
//! resampled to 48 kHz stereo, mixed with per-source volume and streamed
//! as raw f32 PCM into ffmpeg's stdin. Mixer is clock-driven so the audio
//! track never stalls when the system is silent (loopback sends no data then).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use std::collections::VecDeque;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const SAMPLE_RATE: u32 = 48_000;

pub fn output_devices() -> Vec<String> {
    cpal::default_host()
        .output_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default()
}

/// Live peak levels (0..1) for UI meters.
#[derive(Default)]
pub struct Levels {
    system: AtomicU32,
    mic: AtomicU32,
}

impl Levels {
    pub fn system(&self) -> f32 {
        f32::from_bits(self.system.load(Ordering::Relaxed))
    }
    pub fn mic(&self) -> f32 {
        f32::from_bits(self.mic.load(Ordering::Relaxed))
    }
    fn set(&self, mic: bool, v: f32) {
        let slot = if mic { &self.mic } else { &self.system };
        slot.store(v.to_bits(), Ordering::Relaxed);
    }
    pub fn reset(&self) {
        self.set(false, 0.0);
        self.set(true, 0.0);
    }
}

#[derive(Clone)]
pub struct SourceCfg {
    pub device: Option<String>,
    pub volume: f32,
}

pub struct AudioConfig {
    pub system: Option<SourceCfg>,
    pub mic: Option<SourceCfg>,
}

type Queue = Arc<Mutex<VecDeque<f32>>>;

/// Linear resampler for interleaved stereo frames.
struct Resampler {
    step: f64,
    pos: f64,
    prev: [f32; 2],
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Resampler {
            step: from as f64 / to as f64,
            pos: 0.0,
            prev: [0.0; 2],
        }
    }
    fn process(&mut self, input: &[[f32; 2]], out: &mut VecDeque<f32>) {
        let n = input.len();
        if n == 0 {
            return;
        }
        // Virtual sequence: index 0 = prev, 1..=n = input.
        while self.pos < n as f64 {
            let i = self.pos as usize;
            let f = (self.pos - i as f64) as f32;
            let a = if i == 0 { self.prev } else { input[i - 1] };
            let b = input[i];
            out.push_back(a[0] + (b[0] - a[0]) * f);
            out.push_back(a[1] + (b[1] - a[1]) * f);
            self.pos += self.step;
        }
        self.pos -= n as f64;
        self.prev = input[n - 1];
    }
}

fn find_device(host: &cpal::Host, name: &Option<String>, output: bool) -> Option<cpal::Device> {
    match name {
        Some(want) => {
            let mut devs: Box<dyn Iterator<Item = cpal::Device>> = if output {
                Box::new(host.output_devices().ok()?)
            } else {
                Box::new(host.input_devices().ok()?)
            };
            devs.find(|d| d.name().map(|n| &n == want).unwrap_or(false))
        }
        None => {
            if output {
                host.default_output_device()
            } else {
                host.default_input_device()
            }
        }
    }
}

fn open_source(device: &cpal::Device, loopback: bool, queue: Queue) -> Result<cpal::Stream, String> {
    let cfg = if loopback {
        device.default_output_config()
    } else {
        device.default_input_config()
    }
    .map_err(|e| e.to_string())?;
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate().0;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    match fmt {
        cpal::SampleFormat::F32 => build::<f32>(device, &config, channels, rate, queue),
        cpal::SampleFormat::I16 => build::<i16>(device, &config, channels, rate, queue),
        cpal::SampleFormat::U16 => build::<u16>(device, &config, channels, rate, queue),
        cpal::SampleFormat::I32 => build::<i32>(device, &config, channels, rate, queue),
        other => Err(format!("unsupported sample format {other:?}")),
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    rate: u32,
    queue: Queue,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let mut rs = Resampler::new(rate, SAMPLE_RATE);
    let mut frames: Vec<[f32; 2]> = Vec::with_capacity(4096);
    let cap = SAMPLE_RATE as usize * 2; // 1 s of stereo
    let stream = device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                frames.clear();
                for ch in data.chunks(channels.max(1)) {
                    let l = f32::from_sample(ch[0]);
                    let r = if ch.len() > 1 { f32::from_sample(ch[1]) } else { l };
                    frames.push([l, r]);
                }
                let mut q = queue.lock().unwrap();
                rs.process(&frames, &mut q);
                if q.len() > cap {
                    let excess = q.len() - cap;
                    q.drain(..excess);
                }
            },
            |e| eprintln!("audio stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

struct Source {
    queue: Queue,
    volume: f32,
    mic: bool,
    primed: bool,
}

/// Spawn the capture+mix thread. Streams live on this thread (cpal streams are !Send).
pub fn spawn<W: Write + Send + 'static>(
    cfg: AudioConfig,
    mut sink: W,
    stop: Arc<AtomicBool>,
    levels: Arc<Levels>,
    errors: Arc<Mutex<Vec<String>>>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("audio-mixer".into())
        .spawn(move || {
            let host = cpal::default_host();
            let mut streams = Vec::new();
            let mut sources = Vec::new();
            for (src, mic) in [(cfg.system, false), (cfg.mic, true)] {
                let Some(src) = src else { continue };
                let label = if mic { "Microphone" } else { "System audio" };
                let queue: Queue = Arc::new(Mutex::new(VecDeque::new()));
                let res = find_device(&host, &src.device, !mic)
                    .ok_or_else(|| "device not found".to_string())
                    .and_then(|d| open_source(&d, !mic, queue.clone()));
                match res {
                    Ok(s) => {
                        streams.push(s);
                        sources.push(Source {
                            queue,
                            volume: src.volume,
                            mic,
                            primed: false,
                        });
                    }
                    Err(e) => errors.lock().unwrap().push(format!("{label}: {e}")),
                }
            }

            let prime = (SAMPLE_RATE as usize / 25) * 2; // 40 ms jitter buffer
            let max_q = (SAMPLE_RATE as usize / 4) * 2; // >250 ms queued = drift, drop
            let start = Instant::now();
            let mut written: u64 = 0;
            let mut mix: Vec<f32> = Vec::new();
            let mut bytes: Vec<u8> = Vec::new();

            while !stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
                let due = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
                if due <= written {
                    continue;
                }
                let want = ((due - written) as usize) * 2;
                mix.clear();
                mix.resize(want, 0.0);
                for src in sources.iter_mut() {
                    let mut q = src.queue.lock().unwrap();
                    if !src.primed {
                        if q.len() >= prime {
                            src.primed = true;
                        } else {
                            levels.set(src.mic, 0.0);
                            continue;
                        }
                    }
                    if q.len() > max_q {
                        let ex = q.len() - prime;
                        q.drain(..ex);
                    }
                    let n = q.len().min(want);
                    let mut peak = 0f32;
                    for (i, s) in q.drain(..n).enumerate() {
                        let v = s * src.volume;
                        mix[i] += v;
                        peak = peak.max(v.abs());
                    }
                    if n < want {
                        src.primed = false;
                    }
                    levels.set(src.mic, peak.min(1.0));
                }
                bytes.clear();
                for s in &mix {
                    bytes.extend_from_slice(&s.clamp(-1.0, 1.0).to_le_bytes());
                }
                if sink.write_all(&bytes).is_err() {
                    break;
                }
                written = due;
            }
            levels.reset();
            drop(streams);
        })
        .expect("spawn audio thread")
}
