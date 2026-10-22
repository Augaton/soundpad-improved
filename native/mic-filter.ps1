# Installs, removes or reports the microphone filter (bin\SoundpadImprovedApo.dll): an endpoint
# effect (EFX) that Windows' audio engine runs on each microphone, as Soundpad's UniteFx does.
#   -Action status     any user. Which microphones have the filter.
#   -Action install    administrator. Copies the DLL to Program Files, registers it, and puts it
#                      on every active microphone whose effect slot is free or holds Soundpad's.
#   -Action uninstall  administrator. Gives each microphone back the effect it had before.
# install and uninstall restart the Windows audio service: sound cuts out for a second or two.
# The result is one line of JSON, on stdout or in the -Out file (an elevated run has no stdout).
param(
  [Parameter(Mandatory = $true)][ValidateSet('status', 'install', 'uninstall')][string]$Action,
  [string]$Out
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.Encoding]::UTF8

$Clsid = '{827590FA-FFF1-4967-86E6-97F259A0E755}'
$SoundpadClsid = '{27384E53-9860-0AC3-9519-C60EBCAA2C71}'
$EfxKey = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},7'           # PKEY_FX_EndpointEffectClsid
$CompositeEfxKey = '{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},15' # PKEY_CompositeFX_EndpointEffectClsid
$EfxModesKey = '{d3993a3f-99c2-4402-b5ec-a92a0367664b},7'      # PKEY_EFX_ProcessingModes_Supported_For_Streaming
$DefaultMode = '{C18E2F7E-933D-4965-B7D1-1EEF228D2AF3}'        # AUDIO_SIGNALPROCESSINGMODE_DEFAULT
$NameKey = '{a45c254e-df1c-4efd-8020-67d146a850e0},2'          # "Microphone"
$InterfaceKey = '{b3f8fa53-0004-438e-9003-51a46e139bfc},6'     # "TONOR TD510 Dynamic Mic"
$Capture = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture'
$Backup = 'HKLM:\SOFTWARE\Soundpad Improved\Backup'
$ComKey = "HKLM:\SOFTWARE\Classes\CLSID\$Clsid"
$ApoKey = "HKLM:\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\$Clsid"
$InstallDir = Join-Path $env:ProgramFiles 'Soundpad Improved'
$Dll = Join-Path $InstallDir 'SoundpadImprovedApo.dll'

function Get-Microphones {
  foreach ($ep in Get-ChildItem $Capture) {
    if ((Get-ItemProperty $ep.PSPath).DeviceState -ne 1) { continue } # active only
    $props = Get-ItemProperty "$($ep.PSPath)\Properties" -ErrorAction SilentlyContinue
    $fxPath = "$($ep.PSPath)\FxProperties"
    $fx = Get-ItemProperty $fxPath -ErrorAction SilentlyContinue
    [pscustomobject]@{
      Id        = $ep.PSChildName
      Name      = "$($props.$NameKey) ($($props.$InterfaceKey))" # the label Chromium shows
      FxPath    = $fxPath
      HasFx     = $null -ne $fx
      Efx       = if ($fx) { $fx.$EfxKey } else { $null }
      Composite = if ($fx) { $fx.$CompositeEfxKey } else { $null }
      Modes     = if ($fx) { $fx.$EfxModesKey } else { $null }
    }
  }
}

# Virtual cables are skipped (the sounds would reach them twice), and so is any microphone whose
# manufacturer effect would be lost.
function Test-Eligible($mic) {
  $mic.HasFx -and $mic.Name -notmatch 'CABLE|VB-Audio|Voicemeeter' -and -not $mic.Composite -and
  (-not $mic.Efx -or $mic.Efx -eq $SoundpadClsid -or $mic.Efx -eq $Clsid)
}

function Assert-Admin {
  $id = [Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()
  if (-not $id.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'not-admin' }
}

function Get-Status {
  $mics = @(Get-Microphones)
  @{
    dll       = Test-Path $Dll
    installed = @($mics | Where-Object { $_.Efx -eq $Clsid } | ForEach-Object { $_.Name })
    soundpad  = @($mics | Where-Object { $_.Efx -eq $SoundpadClsid } | ForEach-Object { $_.Name })
    eligible  = @($mics | Where-Object { Test-Eligible $_ } | ForEach-Object { $_.Name })
  }
}

function Install-Filter {
  Assert-Admin
  $src = Join-Path $PSScriptRoot 'bin\SoundpadImprovedApo.dll'
  if (-not (Test-Path $src)) { throw 'not-built' }
  $targets = @(Get-Microphones | Where-Object { Test-Eligible $_ })
  if (-not $targets) { throw 'no-mic' }
  Stop-Service Audiosrv -Force # unloads an older copy of the DLL
  try {
    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    Copy-Item $src $Dll -Force

    New-Item -Force "$ComKey\InprocServer32" | Out-Null
    Set-Item $ComKey 'Soundpad Improved microphone filter'
    Set-Item "$ComKey\InprocServer32" $Dll
    New-ItemProperty "$ComKey\InprocServer32" 'ThreadingModel' -Value 'Both' -PropertyType String -Force | Out-Null

    New-Item -Force $ApoKey | Out-Null
    foreach ($p in @{ FriendlyName = 'Soundpad Improved'; Copyright = 'Soundpad Improved'
        APOInterface0 = '{FD7F2B29-24D0-4B5C-B177-592C39F9CA10}' }.GetEnumerator()) {
      New-ItemProperty $ApoKey $p.Key -Value $p.Value -PropertyType String -Force | Out-Null
    }
    # Same values as the DLL's GetRegistrationProperties. MaxInstances -1 is 0xFFFFFFFF.
    foreach ($p in @{ MajorVersion = 1; MinorVersion = 0; Flags = 14; MinInputConnections = 1; MaxInputConnections = 1
        MinOutputConnections = 1; MaxOutputConnections = 1; MaxInstances = -1; NumAPOInterfaces = 1 }.GetEnumerator()) {
      New-ItemProperty $ApoKey $p.Key -Value $p.Value -PropertyType DWord -Force | Out-Null
    }

    foreach ($mic in $targets) {
      $bk = "$Backup\$($mic.Id)"
      if (-not (Test-Path $bk)) { # the first backup holds the original: a reinstall keeps it
        New-Item -Force $bk | Out-Null
        if ($mic.Efx) { New-ItemProperty $bk 'Efx' -Value $mic.Efx -PropertyType String | Out-Null }
        if ($mic.Modes) { New-ItemProperty $bk 'Modes' -Value $mic.Modes -PropertyType MultiString | Out-Null }
      }
      New-ItemProperty $mic.FxPath $EfxKey -Value $Clsid -PropertyType String -Force | Out-Null
      if (-not $mic.Modes) {
        New-ItemProperty $mic.FxPath $EfxModesKey -Value @($DefaultMode) -PropertyType MultiString -Force | Out-Null
      }
    }
  } finally {
    Start-Service Audiosrv
  }
  Get-Status
}

function Uninstall-Filter {
  Assert-Admin
  Stop-Service Audiosrv -Force # releases the DLL
  try {
    foreach ($bk in @(Get-ChildItem $Backup -ErrorAction SilentlyContinue)) {
      $fxPath = "$Capture\$($bk.PSChildName)\FxProperties"
      $fx = Get-ItemProperty $fxPath -ErrorAction SilentlyContinue
      if ($fx -and $fx.$EfxKey -eq $Clsid) { # if Soundpad took the slot back, leave it
        $saved = Get-ItemProperty $bk.PSPath
        if ($saved.Efx) { New-ItemProperty $fxPath $EfxKey -Value $saved.Efx -PropertyType String -Force | Out-Null }
        else { Remove-ItemProperty $fxPath $EfxKey }
        if ($saved.Modes) { New-ItemProperty $fxPath $EfxModesKey -Value $saved.Modes -PropertyType MultiString -Force | Out-Null }
        elseif ($null -ne $fx.$EfxModesKey) { Remove-ItemProperty $fxPath $EfxModesKey }
      }
      Remove-Item $bk.PSPath -Recurse
    }
    Remove-Item (Split-Path $Backup) -Recurse -ErrorAction SilentlyContinue
    Remove-Item $ComKey -Recurse -ErrorAction SilentlyContinue
    Remove-Item $ApoKey -Recurse -ErrorAction SilentlyContinue
    Remove-Item $InstallDir -Recurse -Force -ErrorAction SilentlyContinue
  } finally {
    Start-Service Audiosrv
  }
  Get-Status
}

try {
  $result = switch ($Action) {
    'status' { Get-Status }
    'install' { Install-Filter }
    'uninstall' { Uninstall-Filter }
  }
  $result.ok = $true
} catch {
  $result = @{ ok = $false; error = "$($_.Exception.Message)" }
}
$json = $result | ConvertTo-Json -Compress
if ($Out) { [IO.File]::WriteAllText($Out, $json, (New-Object Text.UTF8Encoding $false)) } else { $json }
if (-not $result.ok) { exit 1 }
