# Installs the FFmpeg/FFprobe used to generate CI media fixtures on Windows and
# exports their directory as FIXTURE_FFMPEG_DIR for later steps.
#
# Chocolatey's ffmpeg package wraps Gyan's essentials build. When the community
# feed is unavailable, the same build is fetched from Gyan's GitHub release at a
# pinned version and verified against its published SHA-256.
$ErrorActionPreference = 'Stop'
$pinnedVersion = '9.0.2'
$pinnedSha256 = '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba'

function Find-Ffmpeg([string] $root) {
  if (-not (Test-Path -LiteralPath $root -PathType Container)) { return $null }
  Get-ChildItem -LiteralPath $root -Filter ffmpeg.exe -Recurse -File | Select-Object -First 1
}

choco install ffmpeg -y --no-progress
# Use the actual binaries: killing a Chocolatey shim leaves its child alive.
$binary = Find-Ffmpeg (Join-Path $env:ChocolateyInstall 'lib\ffmpeg\tools')
if (-not $binary) {
  Write-Warning "Chocolatey ffmpeg unavailable; using pinned Gyan $pinnedVersion essentials build"
  $archive = Join-Path $env:RUNNER_TEMP "ffmpeg-$pinnedVersion-essentials_build.zip"
  Invoke-WebRequest -Uri "https://github.com/GyanD/codexffmpeg/releases/download/$pinnedVersion/ffmpeg-$pinnedVersion-essentials_build.zip" `
    -OutFile $archive -MaximumRetryCount 3 -RetryIntervalSec 15
  $actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $pinnedSha256) { throw "FFmpeg archive SHA-256 mismatch: $actual" }
  $destination = Join-Path $env:RUNNER_TEMP 'fixture-ffmpeg'
  Expand-Archive -LiteralPath $archive -DestinationPath $destination
  $binary = Find-Ffmpeg $destination
}
if (-not $binary) { throw 'FFmpeg binary not found in the installed package' }
if (-not (Test-Path -LiteralPath (Join-Path $binary.DirectoryName 'ffprobe.exe') -PathType Leaf)) { throw 'FFprobe binary not found beside FFmpeg' }
& $binary.FullName -hide_banner -version | Select-Object -First 1
"FIXTURE_FFMPEG_DIR=$($binary.DirectoryName)" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
