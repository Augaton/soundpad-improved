// File -> 48 kHz stereo PCM, the rate every output and the microphone filter run at.
use std::fs::File;
use std::path::Path;

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Resampler};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::errors::Error;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::model::{analyse, normalize_gain};

pub const RATE: u32 = 48000;
const MAX_AUDIO_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MINUTES: u64 = 20; // a small compressed file can unpack to hours: memory stays bounded
const RATES: std::ops::RangeInclusive<u32> = 8000..=384_000;

/// Decoded sound: interleaved stereo at RATE, kept in 16 bits (half the memory of floats, CD
/// quality), and the gain that evens out its loudness.
pub struct Pcm {
    samples: Vec<i16>,
    pub gain: f32,
}

impl Pcm {
    pub fn new(stereo: &[f32], gain: f32) -> Pcm {
        Pcm {
            samples: stereo
                .iter()
                .map(|&s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16)
                .collect(),
            gain,
        }
    }

    pub fn frame(&self, i: usize) -> [f32; 2] {
        [
            f32::from(self.samples[2 * i]) / 32767.0,
            f32::from(self.samples[2 * i + 1]) / 32767.0,
        ]
    }

    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / f64::from(RATE)
    }

    pub fn bytes(&self) -> usize {
        self.samples.len() * 2
    }
}

pub fn decode(path: &str) -> Result<Pcm, String> {
    if !crate::store::is_audio_path(Path::new(path)) {
        return Err("fichier audio refusé".into());
    }
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_AUDIO_BYTES {
        return Err(format!("fichier trop gros ({} Mo)", size / 1_000_000));
    }
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = Path::new(path).extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(
            &hint,
            mss,
            FormatOptions::default(),
            MetadataOptions::default(),
        )
        .map_err(|_| "format non reconnu".to_string())?;
    let track = format
        .default_track(TrackType::Audio)
        .ok_or("aucune piste audio")?;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or("aucune piste audio")?;
    if params.codec == CODEC_ID_OPUS {
        return Err("le format Opus n’est pas pris en charge".into());
    }
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(params, &AudioDecoderOptions::default())
        .map_err(|_| "codec non pris en charge".to_string())?;
    let track_id = track.id;

    // Stereo from the first packet on, so a surround file does not take more memory.
    let (mut stereo, mut chunk, mut channels, mut rate) =
        (Vec::<f32>::new(), Vec::<f32>::new(), 0usize, 0u32);
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if channels == 0 {
                    channels = buf.spec().channels().count().max(1);
                    rate = buf.spec().rate();
                    if !RATES.contains(&rate) {
                        return Err(format!("fréquence non prise en charge ({rate} Hz)"));
                    }
                }
                if buf.spec().channels().count() != channels {
                    continue; // a mid-stream layout change: not worth supporting
                }
                chunk.resize(buf.samples_interleaved(), 0.0);
                buf.copy_to_slice_interleaved(&mut chunk[..]);
                stereo.extend(to_stereo(&chunk, channels));
                if (stereo.len() / 2) as u64 > MAX_MINUTES * 60 * u64::from(rate) {
                    return Err(format!("son trop long (plus de {MAX_MINUTES} minutes)"));
                }
            }
            Err(Error::DecodeError(_)) => {} // a damaged packet: skip it
            Err(e) => return Err(e.to_string()),
        }
    }
    if stereo.is_empty() || rate == 0 {
        return Err("fichier vide".into());
    }
    let stereo = if rate == RATE {
        stereo
    } else {
        resample(&stereo, rate)?
    };
    let (peak, rms) = analyse(&stereo, 2, RATE);
    Ok(Pcm::new(&stereo, normalize_gain(peak, rms)))
}

fn to_stereo(samples: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        2 => samples.to_vec(),
        1 => samples.iter().flat_map(|&v| [v, v]).collect(),
        // Surround: front left and right only.
        _ => samples
            .chunks_exact(channels)
            .flat_map(|f| [f[0], f[1]])
            .collect(),
    }
}

fn resample(stereo: &[f32], rate: u32) -> Result<Vec<f32>, String> {
    let frames = stereo.len() / 2;
    let mut resampler = Fft::<f32>::new(rate as usize, RATE as usize, 1024, 2, FixedSync::Input)
        .map_err(|e| e.to_string())?;
    let needed = resampler.process_all_needed_output_len(frames);
    let mut out = vec![0f32; needed * 2];
    let input = InterleavedSlice::new(stereo, 2, frames).map_err(|e| e.to_string())?;
    let mut output = InterleavedSlice::new_mut(&mut out, 2, needed).map_err(|e| e.to_string())?;
    let (_, produced) = resampler
        .process_all_into_buffer(&input, &mut output, frames, None)
        .map_err(|e| e.to_string())?;
    out.truncate(produced * 2);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_length_and_pitch() {
        // 1 s of 1 kHz at 44.1 kHz -> 1 s at 48 kHz, same frequency.
        let input: Vec<f32> = (0..44100)
            .flat_map(|i| {
                let v = (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin() * 0.5;
                [v, v]
            })
            .collect();
        let out = resample(&input, 44100).unwrap();
        assert!(
            (out.len() as i64 / 2 - 48000).abs() <= 2,
            "{} frames",
            out.len() / 2
        );
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let crossings = left[4800..43200]
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        let freq = crossings as f32 / 2.0 / 0.8;
        assert!((freq - 1000.0).abs() < 5.0, "{freq} Hz");
    }

    #[test]
    fn refuses_what_is_not_an_audio_file() {
        assert_eq!(
            decode(r"\\.\pipe\x.mp3").err().as_deref(),
            Some("fichier audio refusé")
        );
        assert_eq!(
            decode(r"C:\Windows\win.ini").err().as_deref(),
            Some("fichier audio refusé")
        );
    }

    #[test]
    fn mono_and_surround_become_stereo() {
        assert_eq!(to_stereo(&[1.0, 2.0], 1), vec![1.0, 1.0, 2.0, 2.0]);
        assert_eq!(
            to_stereo(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0], 3),
            vec![1.0, 2.0, 4.0, 5.0]
        );
    }
}
