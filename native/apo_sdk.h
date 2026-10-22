// The parts of the Windows SDK's audioenginebaseapo.h and audiomediatype.h that MinGW lacks.
// Layouts and IIDs follow the SDK; the IIDs were checked against audioeng.dll.
#pragma once
#include <windows.h>
#include <mmreg.h>
#include <audioapotypes.h>

typedef enum APO_FLAG {
  APO_FLAG_NONE = 0,
  APO_FLAG_INPLACE = 1,
  APO_FLAG_SAMPLESPERFRAME_MUST_MATCH = 2,
  APO_FLAG_FRAMESPERSECOND_MUST_MATCH = 4,
  APO_FLAG_BITSPERSAMPLE_MUST_MATCH = 8,
  APO_FLAG_MIXER = 16,
  APO_FLAG_DEFAULT = 14,
} APO_FLAG;

typedef struct APO_REG_PROPERTIES {
  CLSID clsid;
  APO_FLAG Flags;
  WCHAR szFriendlyName[256];
  WCHAR szCopyrightInfo[256];
  UINT32 u32MajorVersion;
  UINT32 u32MinorVersion;
  UINT32 u32MinInputConnections;
  UINT32 u32MaxInputConnections;
  UINT32 u32MinOutputConnections;
  UINT32 u32MaxOutputConnections;
  UINT32 u32MaxInstances;
  UINT32 u32NumAPOInterfaces;
  IID iidAPOInterfaceList[1];
} APO_REG_PROPERTIES;

typedef struct UNCOMPRESSEDAUDIOFORMAT {
  GUID guidFormatType;
  DWORD dwSamplesPerFrame;
  DWORD dwBytesPerSampleContainer;
  DWORD dwValidBitsPerSample;
  FLOAT fFramesPerSecond;
  DWORD dwChannelMask;
} UNCOMPRESSEDAUDIOFORMAT;

struct IAudioMediaType : public IUnknown {
  virtual HRESULT STDMETHODCALLTYPE IsCompressedFormat(BOOL* compressed) = 0;
  virtual HRESULT STDMETHODCALLTYPE IsEqual(IAudioMediaType* other, DWORD* flags) = 0;
  virtual const WAVEFORMATEX* STDMETHODCALLTYPE GetAudioFormat() = 0;
  virtual HRESULT STDMETHODCALLTYPE GetUncompressedAudioFormat(UNCOMPRESSEDAUDIOFORMAT* format) = 0;
};

typedef enum APO_CONNECTION_BUFFER_TYPE {
  APO_CONNECTION_BUFFER_TYPE_ALLOCATED = 0,
  APO_CONNECTION_BUFFER_TYPE_EXTERNAL = 1,
  APO_CONNECTION_BUFFER_TYPE_DEPENDANT = 2,
} APO_CONNECTION_BUFFER_TYPE;

typedef struct APO_CONNECTION_DESCRIPTOR {
  APO_CONNECTION_BUFFER_TYPE Type;
  UINT_PTR pBuffer;
  UINT32 u32MaxFrameCount;
  IAudioMediaType* pFormat;
  UINT32 u32Signature;
} APO_CONNECTION_DESCRIPTOR;

struct IAudioProcessingObject : public IUnknown {
  virtual HRESULT STDMETHODCALLTYPE Reset() = 0;
  virtual HRESULT STDMETHODCALLTYPE GetLatency(HNSTIME* time) = 0;
  virtual HRESULT STDMETHODCALLTYPE GetRegistrationProperties(APO_REG_PROPERTIES** props) = 0;
  virtual HRESULT STDMETHODCALLTYPE Initialize(UINT32 size, BYTE* data) = 0;
  virtual HRESULT STDMETHODCALLTYPE IsInputFormatSupported(IAudioMediaType* opposite, IAudioMediaType* requested,
                                                           IAudioMediaType** supported) = 0;
  virtual HRESULT STDMETHODCALLTYPE IsOutputFormatSupported(IAudioMediaType* opposite, IAudioMediaType* requested,
                                                            IAudioMediaType** supported) = 0;
  virtual HRESULT STDMETHODCALLTYPE GetInputChannelCount(UINT32* count) = 0;
};

struct IAudioProcessingObjectRT : public IUnknown {
  virtual void STDMETHODCALLTYPE APOProcess(UINT32 numIn, APO_CONNECTION_PROPERTY** in, UINT32 numOut,
                                            APO_CONNECTION_PROPERTY** out) = 0;
  virtual UINT32 STDMETHODCALLTYPE CalcInputFrames(UINT32 outputFrames) = 0;
  virtual UINT32 STDMETHODCALLTYPE CalcOutputFrames(UINT32 inputFrames) = 0;
};

struct IAudioProcessingObjectConfiguration : public IUnknown {
  virtual HRESULT STDMETHODCALLTYPE LockForProcess(UINT32 numIn, APO_CONNECTION_DESCRIPTOR** in, UINT32 numOut,
                                                   APO_CONNECTION_DESCRIPTOR** out) = 0;
  virtual HRESULT STDMETHODCALLTYPE UnlockForProcess() = 0;
};

struct IAudioSystemEffects : public IUnknown {};

static const IID IID_IAudioMediaType = {0x4E997F73, 0xB71F, 0x4798, {0x87, 0x3B, 0xED, 0x7D, 0xFC, 0xF1, 0x5B, 0x4D}};
static const IID IID_IAudioProcessingObject = {0xFD7F2B29, 0x24D0, 0x4B5C, {0xB1, 0x77, 0x59, 0x2C, 0x39, 0xF9, 0xCA, 0x10}};
static const IID IID_IAudioProcessingObjectRT = {0x9E1D6A6D, 0xDDBC, 0x4E95, {0xA4, 0xC7, 0xAD, 0x64, 0xBA, 0x37, 0x84, 0x6C}};
static const IID IID_IAudioProcessingObjectConfiguration = {0x0E5ED805, 0xABA6, 0x49C3, {0x8F, 0x9A, 0x2B, 0x8C, 0x88, 0x9C, 0x4F, 0xA8}};
static const IID IID_IAudioSystemEffects = {0x5FA00F27, 0xADD6, 0x499A, {0x8A, 0x9D, 0x6B, 0x98, 0x52, 0x1F, 0xA7, 0x5B}};
static const GUID SUBTYPE_IEEE_FLOAT = {0x00000003, 0x0000, 0x0010, {0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71}};

#define APOERR_FORMAT_NOT_SUPPORTED ((HRESULT)0x887D0003L)
#define APOERR_ALREADY_UNLOCKED ((HRESULT)0x887D0006L)
#define APOERR_NUM_CONNECTIONS_INVALID ((HRESULT)0x887D0007L)
#define APOERR_INVALID_CONNECTION_FORMAT ((HRESULT)0x887D0009L)
#define APOERR_APO_LOCKED ((HRESULT)0x887D000AL)
