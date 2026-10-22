// spi-feed.exe: reads the app's sounds on stdin (float32, stereo, 48 kHz, interleaved) and writes
// them into the ring the microphone APO reads (shared.h). Prints its state on stdout when it
// changes:
//   STATE NOMAP        the filter is not loaded (not installed, or no app records the microphone)
//   STATE IDLE         the ring exists but no microphone has been processed for a second
//   STATE LIVE <rate>  a microphone at <rate> Hz is receiving the sounds
// Exits when stdin closes.
#include <math.h>
#include <stdio.h>
#include <string.h>
#include "shared.h"

static const DWORD kReopenMs = 500;
static const uint64_t kReaderStaleMs = 1000;

static void writeFrames(SpiRing* r, const float* frames, uint32_t n) {
  if (n > SPI_CAPACITY) {
    frames += size_t(n - SPI_CAPACITY) * SPI_CHANNELS;
    n = SPI_CAPACITY;
  }
  int64_t w = r->writePos; // this process is the only writer
  for (uint32_t i = 0; i < n; i++) {
    float* f = &r->data[((w + i) & (SPI_CAPACITY - 1)) * SPI_CHANNELS];
    for (uint32_t c = 0; c < SPI_CHANNELS; c++) {
      float x = frames[i * SPI_CHANNELS + c];
      f[c] = isfinite(x) ? (x > 1.f ? 1.f : (x < -1.f ? -1.f : x)) : 0.f; // the APO trusts the ring
    }
  }
  __atomic_store_n(&r->writePos, w + n, __ATOMIC_RELEASE);
  __atomic_store_n(&r->writerTick, GetTickCount64(), __ATOMIC_RELAXED);
}

int main() {
  HANDLE in = GetStdHandle(STD_INPUT_HANDLE);
  HANDLE mapping = nullptr;
  SpiRing* ring = nullptr;
  uint64_t lastOpen = 0;
  char state[32] = "";
  static float buf[16384 + SPI_CHANNELS];
  size_t have = 0; // bytes in buf, less than one frame left over between reads
  const size_t frameBytes = SPI_CHANNELS * sizeof(float);

  for (;;) {
    DWORD got = 0;
    if (!ReadFile(in, reinterpret_cast<char*>(buf) + have, DWORD(sizeof(buf) - frameBytes - have), &got, nullptr) ||
        got == 0)
      break;
    have += got;
    uint32_t frames = uint32_t(have / frameBytes);

    uint64_t now = GetTickCount64();
    if (!ring && now - lastOpen >= kReopenMs) {
      lastOpen = now;
      ring = spiOpenRing(&mapping, false);
    }
    if (ring && spiReady(ring)) writeFrames(ring, buf, frames);

    char next[32];
    if (!ring || !spiReady(ring)) strcpy(next, "NOMAP");
    else if (now - __atomic_load_n(&ring->readerTick, __ATOMIC_RELAXED) > kReaderStaleMs) strcpy(next, "IDLE");
    else snprintf(next, sizeof next, "LIVE %u", __atomic_load_n(&ring->readerRate, __ATOMIC_RELAXED));
    if (strcmp(next, state) != 0) {
      strcpy(state, next);
      printf("STATE %s\n", state);
      fflush(stdout);
    }

    size_t used = size_t(frames) * frameBytes;
    memmove(buf, reinterpret_cast<char*>(buf) + used, have - used);
    have -= used;
  }
  spiCloseRing(ring, mapping);
  return 0;
}
