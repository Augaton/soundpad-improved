// Microphone filter: an endpoint effect (EFX) APO that Windows' audio engine (audiodg.exe) runs
// on the microphone. It adds the sounds the app writes in the shared ring (shared.h) to the
// microphone signal, so every app that records this microphone hears them, the way Soundpad does.
//
// APOProcess runs on the real-time audio thread: it never blocks, allocates or waits. Without a
// live feeder it copies the microphone through untouched.
#include <new>
#include <string.h>
#include "apo_sdk.h"
#include "shared.h"

// {827590FA-FFF1-4967-86E6-97F259A0E755}, also written by mic-filter.ps1.
static const CLSID CLSID_SpiApo = {0x827590FA, 0xFFF1, 0x4967, {0x86, 0xE6, 0x97, 0xF2, 0x59, 0xA0, 0xE7, 0x55}};

static const APO_REG_PROPERTIES kRegProperties = {
    CLSID_SpiApo, APO_FLAG_DEFAULT, L"Soundpad Improved", L"Soundpad Improved", 1, 0, 1, 1, 1, 1, 0xFFFFFFFF, 1,
    {IID_IAudioProcessingObject}};

static const int64_t kTargetFrames = SPI_RATE * 60 / 1000;      // ring latency aimed for
static const int64_t kMaxExcessFrames = SPI_RATE * 200 / 1000;  // beyond target + this, skip ahead
static const uint64_t kStaleMs = 250;                           // feeder silent this long: pass through
static const double kFillSmoothing = 0.02;                      // per buffer (~10 ms)
static const double kMaxSkew = 0.005;                           // playback speed correction, at most 0.5%

static LONG g_objects = 0;
static LONG g_locks = 0;

static float clampSample(float x) { return x > 1.f ? 1.f : (x < -1.f ? -1.f : x); }

static bool floatFormat(IAudioMediaType* type, UNCOMPRESSEDAUDIOFORMAT* f) {
  return type && SUCCEEDED(type->GetUncompressedAudioFormat(f)) && IsEqualGUID(f->guidFormatType, SUBTYPE_IEEE_FLOAT) &&
         f->dwBytesPerSampleContainer == 4 && f->dwSamplesPerFrame >= 1 && f->dwSamplesPerFrame <= 32 &&
         f->fFramesPerSecond >= 8000.f && f->fFramesPerSecond <= 768000.f;
}

// Input and output must be the same float format: the filter adds to the signal, it never converts it.
static HRESULT checkFormat(IAudioMediaType* opposite, IAudioMediaType* requested, IAudioMediaType** supported) {
  if (!requested || !supported) return E_POINTER;
  *supported = nullptr;
  UNCOMPRESSEDAUDIOFORMAT f, o;
  if (!floatFormat(requested, &f)) return APOERR_FORMAT_NOT_SUPPORTED;
  if (opposite && (!floatFormat(opposite, &o) || o.dwSamplesPerFrame != f.dwSamplesPerFrame ||
                   o.fFramesPerSecond != f.fFramesPerSecond))
    return APOERR_FORMAT_NOT_SUPPORTED;
  requested->AddRef();
  *supported = requested;
  return S_OK;
}

class Apo final : public IAudioProcessingObject,
                  public IAudioProcessingObjectRT,
                  public IAudioProcessingObjectConfiguration,
                  public IAudioSystemEffects {
 public:
  // Non-delegating IUnknown, so the audio engine may aggregate the object.
  struct Inner final : public IUnknown {
    Apo* self = nullptr;
    LONG refs = 1;
    STDMETHODIMP QueryInterface(REFIID iid, void** ppv) override {
      if (!ppv) return E_POINTER;
      if (IsEqualIID(iid, IID_IUnknown)) *ppv = static_cast<IUnknown*>(this);
      else if (IsEqualIID(iid, IID_IAudioProcessingObject)) *ppv = static_cast<IAudioProcessingObject*>(self);
      else if (IsEqualIID(iid, IID_IAudioProcessingObjectRT)) *ppv = static_cast<IAudioProcessingObjectRT*>(self);
      else if (IsEqualIID(iid, IID_IAudioProcessingObjectConfiguration))
        *ppv = static_cast<IAudioProcessingObjectConfiguration*>(self);
      else if (IsEqualIID(iid, IID_IAudioSystemEffects)) *ppv = static_cast<IAudioSystemEffects*>(self);
      else {
        *ppv = nullptr;
        return E_NOINTERFACE;
      }
      static_cast<IUnknown*>(*ppv)->AddRef();
      return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refs); }
    STDMETHODIMP_(ULONG) Release() override {
      LONG n = InterlockedDecrement(&refs);
      if (n == 0) delete self;
      return n;
    }
  } inner;

  explicit Apo(IUnknown* outer) : outer_(outer ? outer : &inner) {
    inner.self = this;
    InterlockedIncrement(&g_objects);
  }
  ~Apo() {
    spiCloseRing(ring_, mapping_);
    InterlockedDecrement(&g_objects);
  }

  STDMETHODIMP QueryInterface(REFIID iid, void** ppv) override { return outer_->QueryInterface(iid, ppv); }
  STDMETHODIMP_(ULONG) AddRef() override { return outer_->AddRef(); }
  STDMETHODIMP_(ULONG) Release() override { return outer_->Release(); }

  // ----- IAudioProcessingObject -----

  STDMETHODIMP Reset() override {
    primed_ = false;
    return S_OK;
  }
  STDMETHODIMP GetLatency(HNSTIME* time) override {
    if (!time) return E_POINTER;
    *time = 0; // the microphone itself is not delayed
    return S_OK;
  }
  STDMETHODIMP GetRegistrationProperties(APO_REG_PROPERTIES** props) override {
    if (!props) return E_POINTER;
    *props = static_cast<APO_REG_PROPERTIES*>(CoTaskMemAlloc(sizeof(APO_REG_PROPERTIES)));
    if (!*props) return E_OUTOFMEMORY;
    **props = kRegProperties;
    return S_OK;
  }
  STDMETHODIMP Initialize(UINT32, BYTE*) override { return S_OK; }
  STDMETHODIMP IsInputFormatSupported(IAudioMediaType* opposite, IAudioMediaType* requested,
                                      IAudioMediaType** supported) override {
    return checkFormat(opposite, requested, supported);
  }
  STDMETHODIMP IsOutputFormatSupported(IAudioMediaType* opposite, IAudioMediaType* requested,
                                       IAudioMediaType** supported) override {
    return checkFormat(opposite, requested, supported);
  }
  STDMETHODIMP GetInputChannelCount(UINT32* count) override {
    if (!count) return E_POINTER;
    *count = channels_;
    return S_OK;
  }

  // ----- IAudioProcessingObjectConfiguration -----

  STDMETHODIMP LockForProcess(UINT32 numIn, APO_CONNECTION_DESCRIPTOR** in, UINT32 numOut,
                              APO_CONNECTION_DESCRIPTOR** out) override {
    if (locked_) return APOERR_APO_LOCKED;
    if (numIn != 1 || numOut != 1 || !in || !out || !in[0] || !out[0]) return APOERR_NUM_CONNECTIONS_INVALID;
    UNCOMPRESSEDAUDIOFORMAT fi, fo;
    if (!floatFormat(in[0]->pFormat, &fi) || !floatFormat(out[0]->pFormat, &fo) ||
        fi.dwSamplesPerFrame != fo.dwSamplesPerFrame || fi.fFramesPerSecond != fo.fFramesPerSecond)
      return APOERR_INVALID_CONNECTION_FORMAT;
    channels_ = fi.dwSamplesPerFrame;
    rate_ = fi.fFramesPerSecond;
    maxFrames_ = in[0]->u32MaxFrameCount < out[0]->u32MaxFrameCount ? in[0]->u32MaxFrameCount : out[0]->u32MaxFrameCount;
    if (!ring_) ring_ = spiOpenRing(&mapping_, true); // null: the microphone just passes through
    primed_ = false;
    locked_ = true;
    return S_OK;
  }
  STDMETHODIMP UnlockForProcess() override {
    if (!locked_) return APOERR_ALREADY_UNLOCKED;
    locked_ = false;
    return S_OK;
  }

  // ----- IAudioProcessingObjectRT -----

  STDMETHODIMP_(void) APOProcess(UINT32 numIn, APO_CONNECTION_PROPERTY** in, UINT32 numOut,
                                 APO_CONNECTION_PROPERTY** out) override {
    if (!locked_ || numIn < 1 || numOut < 1 || !in[0] || !out[0]) return;
    APO_CONNECTION_PROPERTY* src = in[0];
    APO_CONNECTION_PROPERTY* dst = out[0];
    UINT32 frames = src->u32ValidFrameCount < maxFrames_ ? src->u32ValidFrameCount : maxFrames_;
    float* s = reinterpret_cast<float*>(src->pBuffer);
    float* d = reinterpret_cast<float*>(dst->pBuffer);
    size_t bytes = size_t(frames) * channels_ * sizeof(float);
    bool silent = src->u32BufferFlags == BUFFER_SILENT;
    if (silent) memset(d, 0, bytes);
    else if (d != s) memcpy(d, s, bytes);
    bool mixed = ring_ && mix(d, frames);
    dst->u32ValidFrameCount = frames;
    dst->u32BufferFlags = silent && !mixed ? BUFFER_SILENT : BUFFER_VALID;
  }
  STDMETHODIMP_(UINT32) CalcInputFrames(UINT32 frames) override { return frames; }
  STDMETHODIMP_(UINT32) CalcOutputFrames(UINT32 frames) override { return frames; }

 private:
  // Adds the ring's sounds to `d`. The ring runs at SPI_RATE on the app's clock, the microphone
  // at rate_ on its own: a linear resampler reads it, a little faster when the ring fills up and
  // slower when it drains, so the two clocks never drift apart.
  bool mix(float* d, UINT32 frames) {
    SpiRing* r = ring_;
    if (!spiReady(r)) return false;
    uint64_t now = GetTickCount64();
    __atomic_store_n(&r->readerTick, now, __ATOMIC_RELAXED);
    __atomic_store_n(&r->readerRate, uint32_t(rate_), __ATOMIC_RELAXED);
    int64_t w = __atomic_load_n(&r->writePos, __ATOMIC_ACQUIRE);
    if (now - __atomic_load_n(&r->writerTick, __ATOMIC_RELAXED) > kStaleMs) {
      primed_ = false;
      return false;
    }
    if (!primed_) {
      if (w - dryAt_ < kTargetFrames) return false; // wait for a full target of new sound
      pos_ = double(w - kTargetFrames);
      avgFill_ = double(kTargetFrames);
      primed_ = true;
    }
    double fill = double(w) - pos_;
    if (fill > double(kTargetFrames + kMaxExcessFrames)) { // fell far behind: skip ahead
      pos_ = double(w - kTargetFrames);
      avgFill_ = fill = double(kTargetFrames);
    }
    avgFill_ += (fill - avgFill_) * kFillSmoothing;
    double skew = (avgFill_ - kTargetFrames) / kTargetFrames;
    skew = skew > 1 ? 1 : (skew < -1 ? -1 : skew);
    double step = SPI_RATE / double(rate_) * (1 + skew * kMaxSkew);
    const int64_t mask = SPI_CAPACITY - 1;
    for (UINT32 i = 0; i < frames; i++, pos_ += step) {
      int64_t k = int64_t(pos_);
      if (k + 1 >= w) { // ran dry: start over once a full target of new sound has arrived
        primed_ = false;
        dryAt_ = w;
        break;
      }
      float t = float(pos_ - double(k));
      const float* a = &r->data[(k & mask) * SPI_CHANNELS];
      const float* b = &r->data[((k + 1) & mask) * SPI_CHANNELS];
      float left = a[0] + (b[0] - a[0]) * t;
      float right = a[1] + (b[1] - a[1]) * t;
      float* f = d + size_t(i) * channels_;
      if (channels_ == 1) {
        f[0] = clampSample(f[0] + 0.5f * (left + right));
      } else {
        f[0] = clampSample(f[0] + left);
        f[1] = clampSample(f[1] + right);
      }
    }
    return true;
  }

  IUnknown* outer_;
  SpiRing* ring_ = nullptr;
  HANDLE mapping_ = nullptr;
  bool locked_ = false;
  UINT32 channels_ = 0;
  float rate_ = 48000.f;
  UINT32 maxFrames_ = 0;
  bool primed_ = false;
  int64_t dryAt_ = 0; // writePos when the ring last ran dry
  double pos_ = 0;
  double avgFill_ = 0;
};

class Factory final : public IClassFactory {
 public:
  STDMETHODIMP QueryInterface(REFIID iid, void** ppv) override {
    if (!ppv) return E_POINTER;
    if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, IID_IClassFactory)) {
      *ppv = static_cast<IClassFactory*>(this);
      return S_OK;
    }
    *ppv = nullptr;
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return 2; } // static object
  STDMETHODIMP_(ULONG) Release() override { return 1; }
  STDMETHODIMP CreateInstance(IUnknown* outer, REFIID iid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (outer && !IsEqualIID(iid, IID_IUnknown)) return CLASS_E_NOAGGREGATION;
    Apo* apo = new (std::nothrow) Apo(outer);
    if (!apo) return E_OUTOFMEMORY;
    HRESULT hr = apo->inner.QueryInterface(iid, ppv);
    apo->inner.Release(); // drop the construction reference
    return hr;
  }
  STDMETHODIMP LockServer(BOOL lock) override {
    if (lock) InterlockedIncrement(&g_locks);
    else InterlockedDecrement(&g_locks);
    return S_OK;
  }
};

static Factory g_factory;

extern "C" HRESULT __stdcall DllGetClassObject(REFCLSID clsid, REFIID iid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (!IsEqualCLSID(clsid, CLSID_SpiApo)) return CLASS_E_CLASSNOTAVAILABLE;
  return g_factory.QueryInterface(iid, ppv);
}

extern "C" HRESULT __stdcall DllCanUnloadNow() { return g_objects == 0 && g_locks == 0 ? S_OK : S_FALSE; }
