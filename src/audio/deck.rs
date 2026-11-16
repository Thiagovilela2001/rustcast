//! Decode any file (Symphonia) to interleaved stereo f32 @ 44.1k (Rubato when needed).
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use std::collections::VecDeque;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, Decoder, DecoderOptions};
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::{FormatOptions, FormatReader};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

pub const RATE: u32 = 44_100;
const CHUNK: usize = 1024;

pub struct Deck {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track_id: u32,
    resampler: Option<Fft<f32>>,
    pending: Vec<f32>,  // stereo @ source rate, waiting for resampler
    out: VecDeque<f32>, // stereo @ 44.1k, ready
    total_frames: Option<u64>,
    played: u64,
    eof: bool,
    pub title: String,
    pub media_id: i64,
    gain: f32,
    /// cue-out set: stop exactly at total_frames instead of decoder EOF
    hard_end: bool,
}

impl Deck {
    pub fn open(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mss = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("no audio track")?;
        let src_rate = track
            .codec_params
            .sample_rate
            .ok_or("unknown sample rate")?;
        let resampler = (src_rate != RATE)
            .then(|| Fft::new(src_rate as usize, RATE as usize, CHUNK, 2, FixedSync::Input))
            .transpose()?;
        Ok(Self {
            decoder: symphonia::default::get_codecs()
                .make(&track.codec_params, &DecoderOptions::default())?,
            track_id: track.id,
            total_frames: track
                .codec_params
                .n_frames
                .map(|n| n * RATE as u64 / src_rate as u64),
            format,
            resampler,
            pending: Vec::new(),
            out: VecDeque::new(),
            played: 0,
            eof: false,
            title: path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
            media_id: 0,
            gain: 1.0,
            hard_end: false,
        })
    }

    /// Apply analyzer results: linear gain from dB, skip leading silence, stop at cue-out.
    pub fn apply_cues(&mut self, gain_db: f32, cue_in_ms: i64, cue_out_ms: Option<i64>) {
        self.gain = 10f32.powf(gain_db / 20.0);
        let to_frames = |ms: i64| (ms.max(0) as u64) * RATE as u64 / 1000;
        if let Some(out) = cue_out_ms.map(to_frames)
            && out > to_frames(cue_in_ms)
        {
            self.total_frames = Some(out);
            self.hard_end = true;
        }
        // ponytail: cue-in by decode-and-discard (leading silence is usually < 2 s); seek if that grows
        let mut skip = to_frames(cue_in_ms);
        let mut scratch = vec![0f32; 4096];
        while skip > 0 {
            let want = (skip as usize).min(scratch.len() / 2);
            let got = self.read(&mut scratch[..want * 2]);
            if got == 0 {
                break;
            }
            skip -= got as u64;
        }
    }

    pub fn remaining_frames(&self) -> Option<u64> {
        self.total_frames.map(|t| t.saturating_sub(self.played))
    }

    /// Fill `buf` (interleaved stereo). Returns frames written; fewer than asked = track over.
    pub fn read(&mut self, buf: &mut [f32]) -> usize {
        let mut want = buf.len();
        if self.hard_end {
            want = want.min(self.remaining_frames().unwrap_or(0) as usize * 2);
        }
        while self.out.len() < want && self.refill() {}
        let n = want.min(self.out.len());
        let g = self.gain;
        for (d, s) in buf.iter_mut().zip(self.out.drain(..n)) {
            *d = s * g;
        }
        buf[n..].fill(0.0);
        self.played += n as u64 / 2;
        n / 2
    }

    fn refill(&mut self) -> bool {
        if self.eof {
            return false;
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(_) => {
                    // EOF or broken stream: flush resampler tail and stop
                    self.eof = true;
                    if let Some(r) = &mut self.resampler
                        && !self.pending.is_empty()
                    {
                        // ponytail: zero-pad final partial chunk (<25ms silence), not delay-trimmed
                        self.pending.resize(r.input_frames_next() * 2, 0.0);
                        Self::resample(r, &mut self.pending, &mut self.out);
                    }
                    return !self.out.is_empty();
                }
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymErr::DecodeError(_)) => continue, // skip corrupt frame
                Err(_) => {
                    self.eof = true;
                    return !self.out.is_empty();
                }
            };
            let spec = *decoded.spec();
            let ch = spec.channels.count();
            let mut sb = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
            sb.copy_interleaved_ref(decoded);
            let dst = if self.resampler.is_some() {
                &mut self.pending
            } else {
                &mut Vec::new()
            };
            for f in sb.samples().chunks_exact(ch) {
                let (l, r) = if ch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
                dst.extend([l, r]);
            }
            match &mut self.resampler {
                Some(r) => Self::resample(r, &mut self.pending, &mut self.out),
                None => self.out.extend(dst.drain(..)),
            }
            return true;
        }
    }

    fn resample(r: &mut Fft<f32>, pending: &mut Vec<f32>, out: &mut VecDeque<f32>) {
        loop {
            let need = r.input_frames_next();
            if pending.len() < need * 2 {
                return;
            }
            let input = InterleavedSlice::new(&pending[..need * 2], 2, need).unwrap();
            if let Ok(o) = r.process(&input, None) {
                out.extend(o.take_data());
            }
            pending.drain(..need * 2);
        }
    }
}
