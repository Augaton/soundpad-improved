// End-to-end test of the microphone filter, as a normal user: loads the APO DLL the way the
// audio engine does, writes a 1 kHz sine in real time into a private ring (SPI_MIC_SHM), the
// way the app does, and checks what comes out of the "microphone".
//   test_apo.exe <dir with SoundpadImprovedApo.dll>
// Listen mode stands in for audiodg.exe while the real app feeds the ring: it creates the ring
// <name> (start it first, with the app's SPI_MIC_SHM), runs a 48 kHz mono "microphone" for
// <seconds> and prints the level of the sounds it received, per half second.
//   test_apo.exe <dir> --listen <name> <seconds>
#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <string>
#include <vector>
#include "apo_sdk.h"
#include "shared.h"
#include <timeapi.h>

static int failures = 0;
#define CHECK(cond, ...)                         \
  do {                                           \
    if (!(cond)) {                               \
      failures++;                                \
      printf("ECHEC %s:%d  ", __FILE__, __LINE__); \
      printf(__VA_ARGS__);                       \
      printf("\n");                              \
    }                                            \
  } while (0)

static const CLSID CLSID_SpiApo = {0x827590FA, 0xFFF1, 0x4967, {0x86, 0xE6, 0x97, 0xF2, 0x59, 0xA0, 0xE7, 0x55}};
static const double kPi = 3.14159265358979323846;

class MediaType final : public IAudioMediaType {
 public:
  MediaType(GUID subtype, DWORD channels, float rate) {
    f_ = {subtype, channels, 4, 32, rate, 0};
  }
  STDMETHODIMP QueryInterface(REFIID iid, void** ppv) override {
    if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, IID_IAudioMediaType)) {
      *ppv = this;
      AddRef();
      return S_OK;
    }
    *ppv = nullptr;
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return ++refs_; }
  STDMETHODIMP_(ULONG) Release() override { return --refs_; } // lives on the stack
  STDMETHODIMP IsCompressedFormat(BOOL* c) override { *c = FALSE; return S_OK; }
  STDMETHODIMP IsEqual(IAudioMediaType*, DWORD*) override { return E_NOTIMPL; }
  const WAVEFORMATEX* STDMETHODCALLTYPE GetAudioFormat() override { return nullptr; }
  STDMETHODIMP GetUncompressedAudioFormat(UNCOMPRESSEDAUDIOFORMAT* f) override { *f = f_; return S_OK; }
  ULONG refs_ = 1;

 private:
  UNCOMPRESSEDAUDIOFORMAT f_;
};

// The app's side of the ring, in-process: opens it once the filter has created it, writes,
// and notes the microphone rates the filter reports.
struct Feeder {
  HANDLE mapping = nullptr;
  SpiRing* ring = nullptr;
  uint32_t lastRate = 0;
  std::string output; // "LIVE <rate>" each time the reported rate changes
  void write(const std::vector<float>& frames) {
    if (!ring) ring = spiOpenRing(&mapping, false);
    if (ring && spiReady(ring)) spiWrite(ring, frames.data(), uint32_t(frames.size() / SPI_CHANNELS));
  }
  void poll() {
    if (!ring || !spiReady(ring)) return;
    if (GetTickCount64() - __atomic_load_n(&ring->readerTick, __ATOMIC_RELAXED) > 1000) return;
    uint32_t rate = __atomic_load_n(&ring->readerRate, __ATOMIC_RELAXED);
    if (rate != lastRate) {
      lastRate = rate;
      output += "LIVE " + std::to_string(rate) + "\n";
    }
  }
  void stop() {
    spiCloseRing(ring, mapping);
    ring = nullptr;
    mapping = nullptr;
  }
};

static double seconds() {
  static LARGE_INTEGER freq = {}, start = {};
  LARGE_INTEGER now;
  if (!freq.QuadPart) {
    QueryPerformanceFrequency(&freq);
    QueryPerformanceCounter(&start);
  }
  QueryPerformanceCounter(&now);
  return double(now.QuadPart - start.QuadPart) / double(freq.QuadPart);
}

struct Result {
  std::vector<float> extra; // output minus the microphone input, first channel
  std::vector<float> second; // same, second channel (stereo microphones)
  float maxOut = 0;
  int passthroughErrors = 0; // frames that differ from the input while no sound is expected
};

// Runs one APO instance for `duration` seconds of real time. The writer sends a sine at
// `writeRate` frames per second (48000 = in step with the microphone) while `feeding`.
static Result run(IClassFactory* factory, Feeder* feeder, DWORD channels, float rate, float micLevel,
                  double duration, double writeRate, bool feeding) {
  Result res;
  IAudioProcessingObject* apo = nullptr;
  HRESULT hr = factory->CreateInstance(nullptr, IID_IAudioProcessingObject, reinterpret_cast<void**>(&apo));
  CHECK(SUCCEEDED(hr) && apo, "CreateInstance %08lx", hr);
  if (!apo) return res;
  IAudioProcessingObjectRT* rt = nullptr;
  IAudioProcessingObjectConfiguration* cfg = nullptr;
  apo->QueryInterface(IID_IAudioProcessingObjectRT, reinterpret_cast<void**>(&rt));
  apo->QueryInterface(IID_IAudioProcessingObjectConfiguration, reinterpret_cast<void**>(&cfg));
  CHECK(rt && cfg, "interfaces RT / Configuration absentes");
  CHECK(SUCCEEDED(apo->Initialize(0, nullptr)), "Initialize");

  MediaType type(SUBTYPE_IEEE_FLOAT, channels, rate);
  const UINT32 block = UINT32(rate / 100); // 10 ms, like the audio engine
  std::vector<float> inBuf(block * channels), outBuf(block * channels);
  APO_CONNECTION_DESCRIPTOR din = {APO_CONNECTION_BUFFER_TYPE_ALLOCATED, UINT_PTR(inBuf.data()), block, &type, 0};
  APO_CONNECTION_DESCRIPTOR dout = {APO_CONNECTION_BUFFER_TYPE_ALLOCATED, UINT_PTR(outBuf.data()), block, &type, 0};
  APO_CONNECTION_DESCRIPTOR* pin = &din;
  APO_CONNECTION_DESCRIPTOR* pout = &dout;
  hr = cfg->LockForProcess(1, &pin, 1, &pout);
  CHECK(SUCCEEDED(hr), "LockForProcess %08lx", hr);

  const UINT32 chunk = 512;
  double phase = 0;
  int64_t written = 0, processed = 0;
  double t0 = seconds();
  for (;;) {
    double t = seconds() - t0;
    if (t >= duration) break;
    while (feeding && written < int64_t(t * writeRate) + chunk) {
      std::vector<float> pcm(chunk * 2);
      for (UINT32 i = 0; i < chunk; i++) {
        float v = float(0.5 * sin(phase));
        phase += 2 * kPi * 1000 / writeRate;
        pcm[i * 2] = pcm[i * 2 + 1] = v;
      }
      feeder->write(pcm);
      written += chunk;
    }
    while (processed + block <= int64_t(t * rate)) {
      for (auto& x : inBuf) x = micLevel;
      APO_CONNECTION_PROPERTY pi = {UINT_PTR(inBuf.data()), block, BUFFER_VALID, 0};
      APO_CONNECTION_PROPERTY po = {UINT_PTR(outBuf.data()), 0, BUFFER_INVALID, 0};
      APO_CONNECTION_PROPERTY* ppi = &pi;
      APO_CONNECTION_PROPERTY* ppo = &po;
      rt->APOProcess(1, &ppi, 1, &ppo);
      CHECK(po.u32ValidFrameCount == block && po.u32BufferFlags == BUFFER_VALID, "tampon de sortie invalide");
      for (UINT32 i = 0; i < block; i++) {
        float a = outBuf[i * channels];
        res.maxOut = fmaxf(res.maxOut, fabsf(a));
        if (!feeding && a != micLevel) res.passthroughErrors++;
        res.extra.push_back(a - micLevel);
        if (channels > 1) res.second.push_back(outBuf[i * channels + 1] - micLevel);
      }
      processed += block;
    }
    if (feeder) feeder->poll();
    Sleep(1);
  }
  cfg->UnlockForProcess();
  cfg->Release();
  rt->Release();
  CHECK(apo->Release() == 0, "référence perdue sur l'APO");
  return res;
}

// Sine analysis of the samples after `skip`: RMS, frequency from zero crossings, and the number
// of jumps no 1 kHz sine of amplitude 0.5 can make between two samples (a glitch or a resync).
static void analyse(const std::vector<float>& x, size_t skip, float rate, double* rms, double* freq, int* jumps) {
  double sum = 0;
  int crossings = 0;
  *jumps = 0;
  const double maxStep = 0.5 * 2 * kPi * 1000 / rate * 1.5;
  for (size_t i = skip + 1; i < x.size(); i++) {
    sum += double(x[i]) * x[i];
    if ((x[i - 1] < 0) != (x[i] < 0)) crossings++;
    if (fabs(double(x[i]) - x[i - 1]) > maxStep) (*jumps)++;
  }
  size_t n = x.size() > skip + 1 ? x.size() - skip - 1 : 1;
  *rms = sqrt(sum / double(n));
  *freq = crossings / 2.0 / (double(n) / rate);
}

int main(int argc, char** argv) {
  std::string dir = argc > 1 ? argv[1] : ".";
  bool listen = argc > 4 && strcmp(argv[2], "--listen") == 0;
  char name[64];
  snprintf(name, sizeof name, "Local\\SpiMicTest%lu", GetCurrentProcessId());
  SetEnvironmentVariableA("SPI_MIC_SHM", listen ? argv[3] : name); // read by the DLL and the feeder
  CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  timeBeginPeriod(1); // Sleep(1) may last 15 ms otherwise (CI machines), too coarse for real time

  HMODULE dll = LoadLibraryA((dir + "\\SoundpadImprovedApo.dll").c_str());
  CHECK(dll, "DLL introuvable (%lu)", GetLastError());
  if (!dll) return 1;
  auto getClass = reinterpret_cast<HRESULT(__stdcall*)(REFCLSID, REFIID, void**)>(reinterpret_cast<void*>(GetProcAddress(dll, "DllGetClassObject")));
  auto canUnload = reinterpret_cast<HRESULT(__stdcall*)()>(reinterpret_cast<void*>(GetProcAddress(dll, "DllCanUnloadNow")));
  CHECK(getClass && canUnload, "exports DllGetClassObject / DllCanUnloadNow absents");
  IClassFactory* factory = nullptr;
  CHECK(SUCCEEDED(getClass(CLSID_SpiApo, IID_IClassFactory, reinterpret_cast<void**>(&factory))), "fabrique");
  if (listen) {
    Result r = run(factory, nullptr, 1, 48000, 0.f, atof(argv[4]), 0, false);
    printf("niveau reçu par demi-seconde :");
    for (size_t i = 0; i + 24000 <= r.extra.size(); i += 24000) {
      double sum = 0;
      for (size_t k = i; k < i + 24000; k++) sum += double(r.extra[k]) * r.extra[k];
      printf(" %.3f", sqrt(sum / 24000));
    }
    printf("\n");
    return 0;
  }
  IClassFactory* wrong = nullptr;
  CHECK(getClass(IID_IUnknown, IID_IClassFactory, reinterpret_cast<void**>(&wrong)) == CLASS_E_CLASSNOTAVAILABLE,
        "un autre CLSID doit être refusé");

  // Registration properties and format negotiation.
  {
    IAudioProcessingObject* apo = nullptr;
    factory->CreateInstance(nullptr, IID_IAudioProcessingObject, reinterpret_cast<void**>(&apo));
    APO_REG_PROPERTIES* props = nullptr;
    CHECK(SUCCEEDED(apo->GetRegistrationProperties(&props)) && IsEqualCLSID(props->clsid, CLSID_SpiApo) &&
              props->Flags == APO_FLAG_DEFAULT && props->u32NumAPOInterfaces == 1,
          "GetRegistrationProperties");
    CoTaskMemFree(props);
    IUnknown* sfx = nullptr;
    CHECK(SUCCEEDED(apo->QueryInterface(IID_IAudioSystemEffects, reinterpret_cast<void**>(&sfx))), "IAudioSystemEffects");
    if (sfx) sfx->Release();
    MediaType f48(SUBTYPE_IEEE_FLOAT, 1, 48000), f44(SUBTYPE_IEEE_FLOAT, 1, 44100);
    MediaType pcm({0x00000001, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71}}, 1, 48000);
    IAudioMediaType* got = nullptr;
    CHECK(apo->IsInputFormatSupported(nullptr, &f48, &got) == S_OK && got == &f48, "float 48 kHz accepté");
    if (got) got->Release();
    CHECK(apo->IsOutputFormatSupported(&f48, &f44, &got) == APOERR_FORMAT_NOT_SUPPORTED && !got, "fréquences différentes refusées");
    CHECK(apo->IsInputFormatSupported(nullptr, &pcm, &got) == APOERR_FORMAT_NOT_SUPPORTED && !got, "PCM entier refusé");
    CHECK(apo->Release() == 0, "référence perdue");
  }
  // Aggregation: only IUnknown may be asked for.
  {
    MediaType outer(SUBTYPE_IEEE_FLOAT, 1, 48000); // any IUnknown will do as the outer object
    void* p = nullptr;
    CHECK(factory->CreateInstance(&outer, IID_IAudioProcessingObject, &p) == CLASS_E_NOAGGREGATION, "agrégation");
    IUnknown* inner = nullptr;
    CHECK(SUCCEEDED(factory->CreateInstance(&outer, IID_IUnknown, reinterpret_cast<void**>(&inner))) && inner, "agrégation IUnknown");
    if (inner) CHECK(inner->Release() == 0, "objet agrégé non libéré");
  }

  Feeder feeder;
  double rms, freq;
  int jumps;

  // 1. Feeder silent: the microphone passes through bit for bit.
  Result r = run(factory, &feeder, 1, 48000, 0.25f, 0.3, 48000, false);
  CHECK(r.passthroughErrors == 0, "micro modifié sans son : %d échantillons", r.passthroughErrors);

  // 2. Mono microphone at 48 kHz: the sine is added on top of the voice.
  r = run(factory, &feeder, 1, 48000, 0.25f, 1.5, 48000, true);
  analyse(r.extra, 48000 / 2, 48000, &rms, &freq, &jumps);
  printf("mono 48 kHz      : RMS %.3f  %.0f Hz  sauts %d\n", rms, freq, jumps);
  CHECK(fabs(rms - 0.3536) < 0.03, "niveau du son ajouté %.3f", rms);
  CHECK(fabs(freq - 1000) < 15, "fréquence %.1f Hz", freq);
  CHECK(jumps == 0, "%d discontinuités", jumps);

  // 3. Stereo microphone at 44.1 kHz: resampled, same pitch, on both channels.
  r = run(factory, &feeder, 2, 44100, 0.f, 1.5, 48000, true);
  analyse(r.extra, 44100 / 2, 44100, &rms, &freq, &jumps);
  double rms2, freq2;
  int jumps2;
  analyse(r.second, 44100 / 2, 44100, &rms2, &freq2, &jumps2);
  printf("stéréo 44,1 kHz  : RMS %.3f / %.3f  %.0f Hz  sauts %d\n", rms, rms2, freq, jumps + jumps2);
  CHECK(fabs(rms - 0.3536) < 0.03 && fabs(rms2 - 0.3536) < 0.03, "niveau stéréo %.3f / %.3f", rms, rms2);
  CHECK(fabs(freq - 1000) < 15, "fréquence rééchantillonnée %.1f Hz", freq);
  CHECK(jumps + jumps2 == 0, "%d discontinuités", jumps + jumps2);

  // 4. App clock 0.3% fast (far worse than real devices): absorbed without a glitch.
  r = run(factory, &feeder, 1, 48000, 0.f, 4, 48000 * 1.003, true);
  analyse(r.extra, 48000 / 2, 48000, &rms, &freq, &jumps);
  printf("horloge +0,3 %%   : RMS %.3f  %.0f Hz  sauts %d\n", rms, freq, jumps);
  CHECK(jumps == 0, "dérive d'horloge : %d discontinuités", jumps);

  // 5. Loud voice plus sound: clipped to full scale, never beyond.
  r = run(factory, &feeder, 1, 48000, 0.9f, 0.6, 48000, true);
  printf("saturation       : crête %.3f\n", r.maxOut);
  CHECK(r.maxOut <= 1.f && r.maxOut > 0.99f, "crête %.3f", r.maxOut);

  // 6. Feeder stops: what is buffered plays once (no looping of the tail), then the plain
  //    microphone, also for a new instance.
  feeder.poll();
  r = run(factory, &feeder, 1, 48000, 0.25f, 0.3, 48000, false);
  CHECK(r.passthroughErrors > 0 && r.passthroughErrors <= 2880 + 512, "fin du son : %d échantillons modifiés",
        r.passthroughErrors);
  r = run(factory, &feeder, 1, 48000, 0.25f, 0.3, 48000, false);
  CHECK(r.passthroughErrors == 0, "micro modifié après l'arrêt : %d échantillons", r.passthroughErrors);

  feeder.stop();
  printf("micro vu par l'appli :\n%s", feeder.output.c_str());
  CHECK(feeder.output.find("LIVE 48000") != std::string::npos, "micro à 48000 Hz non signalé");
  CHECK(feeder.output.find("LIVE 44100") != std::string::npos, "micro à 44100 Hz non signalé");

  CHECK(canUnload() == S_OK, "DllCanUnloadNow : objets encore vivants");
  printf(failures ? "\n%d ECHEC(S)\n" : "\nOK\n", failures);
  return failures ? 1 : 0;
}
