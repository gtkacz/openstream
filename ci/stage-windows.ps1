# Stages the Windows build into one directory: the exe, the FFmpeg DLLs it loads, the crash-dump
# helper, and licenses.
param(
  [Parameter(Mandatory = $true)][string]$Destination
)
$ErrorActionPreference = "Stop"
New-Item -ItemType Directory -Force $Destination | Out-Null
Copy-Item target\release\brp.exe $Destination
foreach ($dll in "avcodec-62.dll", "avutil-60.dll", "swscale-9.dll", "swresample-6.dll") {
  Copy-Item (Join-Path $env:FFMPEG_DIR "bin\$dll") $Destination
}
Copy-Item tools\windows-enable-crash-dumps.reg $Destination
Copy-Item (Join-Path $env:FFMPEG_DIR "LICENSE.txt") (Join-Path $Destination "FFMPEG-LICENSE.txt")
Copy-Item LICENSE (Join-Path $Destination "LICENSE")
