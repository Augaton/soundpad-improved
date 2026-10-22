// Shared-memory ring between spi-feed.exe (writer: the app's sounds) and the microphone APO
// (reader, inside audiodg.exe). One writer, any number of readers, no locks: the writer stores
// the samples, then publishes writePos; a reader never reads at or past writePos.
#pragma once
#include <windows.h>
#include <sddl.h>
#include <stdint.h>

enum : uint32_t {
  SPI_MAGIC = 0x4D495053, // "SPIM"
  SPI_VERSION = 1,
  SPI_RATE = 48000,
  SPI_CHANNELS = 2,
  SPI_CAPACITY = 32768, // frames, a power of two (~680 ms)
};

struct SpiRing {
  uint32_t magic, version, rate, channels, capacity, reserved[3];
  int64_t writePos;    // frames ever written, published after the samples
  uint64_t writerTick; // GetTickCount64() of the last write
  uint64_t readerTick; // GetTickCount64() of the last microphone buffer processed
  uint32_t readerRate; // frame rate of that microphone
  uint32_t reserved2[9];
  float data[SPI_CAPACITY * SPI_CHANNELS]; // interleaved stereo
};

// Creating a section in Global\ needs SeCreateGlobalPrivilege: audiodg.exe has it, a normal
// user does not, so the APO creates the ring and the feeder opens it. SPI_MIC_SHM overrides the
// name for tests ("Local\..." lets both ends run as a normal user).
inline void spiRingName(wchar_t* name, DWORD size) {
  DWORD n = GetEnvironmentVariableW(L"SPI_MIC_SHM", name, size);
  if (n == 0 || n >= size) lstrcpynW(name, L"Global\\SoundpadImprovedMic", size);
}

inline bool spiReady(const SpiRing* r) {
  return __atomic_load_n(&r->magic, __ATOMIC_ACQUIRE) == SPI_MAGIC && r->version == SPI_VERSION &&
         r->rate == SPI_RATE && r->channels == SPI_CHANNELS && r->capacity == SPI_CAPACITY;
}

// Opens the ring, creating it if `create` and it does not exist yet. Null on failure.
inline SpiRing* spiOpenRing(HANDLE* mapping, bool create) {
  wchar_t name[128];
  spiRingName(name, 128);
  bool created = false;
  HANDLE h = OpenFileMappingW(FILE_MAP_READ | FILE_MAP_WRITE, FALSE, name);
  if (!h && create) {
    // SYSTEM, LOCAL SERVICE (audiodg) and administrators: full; the logged-on user: read/write.
    PSECURITY_DESCRIPTOR sd = nullptr;
    ConvertStringSecurityDescriptorToSecurityDescriptorW(
        L"D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;BA)(A;;GRGW;;;IU)S:(ML;;NW;;;ME)", SDDL_REVISION_1, &sd, nullptr);
    SECURITY_ATTRIBUTES sa = {sizeof sa, sd, FALSE};
    h = CreateFileMappingW(INVALID_HANDLE_VALUE, sd ? &sa : nullptr, PAGE_READWRITE, 0, sizeof(SpiRing), name);
    created = h && GetLastError() != ERROR_ALREADY_EXISTS;
    LocalFree(sd);
  }
  if (!h) return nullptr;
  auto* r = static_cast<SpiRing*>(MapViewOfFile(h, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, sizeof(SpiRing)));
  if (!r) {
    CloseHandle(h);
    return nullptr;
  }
  if (created) { // a new section is zero-filled; the magic goes last
    r->version = SPI_VERSION;
    r->rate = SPI_RATE;
    r->channels = SPI_CHANNELS;
    r->capacity = SPI_CAPACITY;
    __atomic_store_n(&r->magic, SPI_MAGIC, __ATOMIC_RELEASE);
  }
  *mapping = h;
  return r;
}

inline void spiCloseRing(SpiRing* r, HANDLE mapping) {
  if (r) UnmapViewOfFile(r);
  if (mapping) CloseHandle(mapping);
}
