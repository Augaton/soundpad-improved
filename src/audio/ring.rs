// Writer side of the shared-memory ring the microphone filter reads (native/shared.h: keep the
// two in step). One writer, no locks: the samples are stored, then writePos is published.
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Memory::{
    FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
    UnmapViewOfFile,
};
use windows::Win32::System::SystemInformation::GetTickCount64;
use windows::core::PCWSTR;

use crate::model::FilterLive;

const MAGIC: u32 = 0x4D49_5053; // "SPIM"
const VERSION: u32 = 1;
pub const RATE: u32 = 48000;
const CHANNELS: u32 = 2;
const CAPACITY: u32 = 32768; // frames, a power of two
const READER_STALE_MS: u64 = 1000;

#[repr(C)]
pub struct Header {
    magic: AtomicU32,
    version: u32,
    rate: u32,
    channels: u32,
    capacity: u32,
    reserved: [u32; 3],
    write_pos: AtomicI64,   // frames ever written, published after the samples
    writer_tick: AtomicU64, // GetTickCount64() of the last write
    reader_tick: AtomicU64, // GetTickCount64() of the last microphone buffer processed
    reader_rate: AtomicU32, // frame rate of that microphone
    reserved2: [u32; 9],
}

pub const SIZE: usize = size_of::<Header>() + (CAPACITY * CHANNELS) as usize * 4;

/// The ring's name: SPI_MIC_SHM overrides it for tests ("Local\..." works without the APO's
/// SeCreateGlobalPrivilege).
pub fn ring_name() -> String {
    std::env::var("SPI_MIC_SHM")
        .ok()
        .filter(|n| !n.is_empty() && n.len() < 128)
        .unwrap_or_else(|| "Global\\SoundpadImprovedMic".into())
}

pub struct Ring {
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
}

// The view is plain shared memory, accessed through atomics and a single writer.
unsafe impl Send for Ring {}

impl Ring {
    /// Opens the ring the filter created. None while the filter is not loaded.
    pub fn open(name: &str) -> Option<Ring> {
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        unsafe {
            let mapping = OpenFileMappingW(
                (FILE_MAP_READ | FILE_MAP_WRITE).0,
                false,
                PCWSTR(wide.as_ptr()),
            )
            .ok()?;
            let view = MapViewOfFile(mapping, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, SIZE);
            if view.Value.is_null() {
                let _ = CloseHandle(mapping);
                return None;
            }
            Some(Ring { mapping, view })
        }
    }

    fn header(&self) -> &Header {
        unsafe { &*(self.view.Value as *const Header) }
    }

    pub fn ready(&self) -> bool {
        let h = self.header();
        h.magic.load(Ordering::Acquire) == MAGIC
            && h.version == VERSION
            && h.rate == RATE
            && h.channels == CHANNELS
            && h.capacity == CAPACITY
    }

    /// Writes interleaved stereo frames. The filter trusts the ring: values are clamped.
    pub fn write(&mut self, frames: &[f32]) {
        let mut frames = &frames[..frames.len() / 2 * 2];
        let cap = CAPACITY as usize;
        if frames.len() / 2 > cap {
            frames = &frames[frames.len() - cap * 2..];
        }
        let data = unsafe { (self.view.Value as *mut u8).add(size_of::<Header>()) as *mut f32 };
        let h = self.header();
        let w = h.write_pos.load(Ordering::Relaxed); // the only writer
        for (i, f) in frames.as_chunks::<2>().0.iter().enumerate() {
            let slot = ((w as usize + i) & (cap - 1)) * 2;
            for (c, &x) in f.iter().enumerate() {
                let x = if x.is_finite() {
                    x.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
                unsafe { data.add(slot + c).write_volatile(x) };
            }
        }
        h.write_pos
            .store(w + (frames.len() / 2) as i64, Ordering::Release);
        h.writer_tick
            .store(unsafe { GetTickCount64() }, Ordering::Relaxed);
    }

    pub fn state(&self) -> FilterLive {
        if !self.ready() {
            return FilterLive::NoMap;
        }
        let h = self.header();
        let now = unsafe { GetTickCount64() };
        if now.saturating_sub(h.reader_tick.load(Ordering::Relaxed)) > READER_STALE_MS {
            FilterLive::Idle
        } else {
            FilterLive::Live(h.reader_rate.load(Ordering::Relaxed))
        }
    }
}

impl Drop for Ring {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(self.view);
            let _ = CloseHandle(self.mapping);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};

    #[test]
    fn layout_matches_shared_h() {
        assert_eq!(size_of::<Header>(), 96);
        assert_eq!(std::mem::offset_of!(Header, write_pos), 32);
        assert_eq!(std::mem::offset_of!(Header, reader_rate), 56);
        assert_eq!(SIZE, 96 + 32768 * 2 * 4);
    }

    #[test]
    fn writes_wrap_clamp_and_publish() {
        let name = format!("Local\\SpiRingTest{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
        // Stands in for the filter: creates the section and fills the header, magic last.
        let section = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                0,
                SIZE as u32,
                PCWSTR(wide.as_ptr()),
            )
        }
        .unwrap();
        let mut ring = Ring::open(&name).expect("open");
        assert!(!ring.ready());
        assert_eq!(ring.state(), FilterLive::NoMap);
        let h = unsafe { &mut *(ring.view.Value as *mut Header) };
        (h.version, h.rate, h.channels, h.capacity) = (VERSION, RATE, CHANNELS, CAPACITY);
        h.magic.store(MAGIC, Ordering::Release);
        assert!(ring.ready());
        assert_eq!(ring.state(), FilterLive::Idle);

        h.write_pos
            .store(i64::from(CAPACITY) - 1, Ordering::Relaxed);
        ring.write(&[0.5, -2.0, f32::NAN, 0.25, 9.0]); // the odd trailing sample is dropped
        let data = unsafe {
            std::slice::from_raw_parts(
                (ring.view.Value as *const u8).add(96) as *const f32,
                (CAPACITY * 2) as usize,
            )
        };
        assert_eq!(&data[(CAPACITY as usize - 1) * 2..], &[0.5, -1.0]);
        assert_eq!(&data[..2], &[0.0, 0.25]);
        assert_eq!(h.write_pos.load(Ordering::Acquire), i64::from(CAPACITY) + 1);

        h.reader_tick
            .store(unsafe { GetTickCount64() }, Ordering::Relaxed);
        h.reader_rate.store(44100, Ordering::Relaxed);
        assert_eq!(ring.state(), FilterLive::Live(44100));
        drop(ring);
        unsafe { CloseHandle(section) }.unwrap();
    }
}
