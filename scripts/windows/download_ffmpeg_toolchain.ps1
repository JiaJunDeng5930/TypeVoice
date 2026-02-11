param(
  [ValidateSet("auto", "all", "windows-x86_64", "linux-x86_64")]
  [string]$Platform = "auto"
)

$ErrorActionPreference = "Stop"

function Fail([string]$msg) {
  Write-Host ("FAIL: " + $msg) -ForegroundColor Red
  exit 1
}

function Info([string]$msg) {
  Write-Host ("INFO: " + $msg) -ForegroundColor Cyan
}

function Resolve-RepoRoot {
  return (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
}

function Get-PlatformAuto {
  if ($env:PROCESSOR_ARCHITECTURE -match "(AMD64|x86_64)") {
    return "windows-x86_64"
  }
  return ""
}

function Get-FileSha256([string]$path) {
  return (Get-FileHash -Algorithm SHA256 -Path $path).Hash.ToLowerInvariant()
}

$RepoRoot = Resolve-RepoRoot
$ManifestPath = Join-Path $RepoRoot "apps\desktop\src-tauri\toolchain\ffmpeg_manifest.json"
if (-not (Test-Path $ManifestPath)) {
  Fail ("manifest missing: " + $ManifestPath)
}

$Manifest = Get-Content -Raw -Path $ManifestPath | ConvertFrom-Json
$targets = @()

switch ($Platform) {
  "all" {
    $targets = @("windows-x86_64", "linux-x86_64")
  }
  "auto" {
    $p = Get-PlatformAuto
    if ([string]::IsNullOrWhiteSpace($p)) {
      Fail "unsupported host platform for auto mode"
    }
    $targets = @($p)
  }
  default {
    $targets = @($Platform)
  }
}

$workRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("typevoice_ffmpeg_" + [guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $workRoot | Out-Null

try {
  foreach ($target in $targets) {
    $spec = $Manifest.platforms.$target
    if ($null -eq $spec) {
      Fail ("platform not found in manifest: " + $target)
    }

    Info ("prepare ffmpeg toolchain for " + $target)

    $workDir = Join-Path $workRoot $target
    $unpackDir = Join-Path $workDir "unpack"
    $archiveExt = switch ($spec.archive_type) {
      "zip" { ".zip" }
      "tar.xz" { ".tar.xz" }
      default { "" }
    }
    $archivePath = Join-Path $workDir ("archive" + $archiveExt)
    New-Item -ItemType Directory -Force -Path $unpackDir | Out-Null

    & curl.exe -L --fail --silent --show-error --output $archivePath $spec.archive_url
    if ($LASTEXITCODE -ne 0) {
      Fail ("download failed for " + $target + " url=" + $spec.archive_url)
    }

    $archiveSha = Get-FileSha256 $archivePath
    if ($archiveSha -ne $spec.archive_sha256.ToLowerInvariant()) {
      Fail ("archive sha256 mismatch for " + $target + " expected=" + $spec.archive_sha256 + " actual=" + $archiveSha)
    }

    switch ($spec.archive_type) {
      "zip" {
        Expand-Archive -Path $archivePath -DestinationPath $unpackDir -Force
        $srcFfmpeg = Join-Path $unpackDir ($spec.archive_root + "\\bin\\" + $spec.ffmpeg_file)
        $srcFfprobe = Join-Path $unpackDir ($spec.archive_root + "\\bin\\" + $spec.ffprobe_file)
      }
      "tar.xz" {
        Push-Location $unpackDir
        try {
          tar -xf $archivePath
          if ($LASTEXITCODE -ne 0) {
            Fail ("tar extraction failed for " + $target)
          }
        } finally {
          Pop-Location
        }
        $srcFfmpeg = Join-Path $unpackDir ($spec.archive_root + "\\" + $spec.ffmpeg_file)
        $srcFfprobe = Join-Path $unpackDir ($spec.archive_root + "\\" + $spec.ffprobe_file)
      }
      default {
        Fail ("unsupported archive_type for " + $target + ": " + $spec.archive_type)
      }
    }

    if (-not (Test-Path $srcFfmpeg)) {
      Fail ("missing extracted ffmpeg: " + $srcFfmpeg)
    }
    if (-not (Test-Path $srcFfprobe)) {
      Fail ("missing extracted ffprobe: " + $srcFfprobe)
    }

    $destDir = Join-Path $RepoRoot ("apps\\desktop\\src-tauri\\toolchain\\bin\\" + $target)
    New-Item -ItemType Directory -Force -Path $destDir | Out-Null

    $destFfmpeg = Join-Path $destDir $spec.ffmpeg_file
    $destFfprobe = Join-Path $destDir $spec.ffprobe_file

    Copy-Item -Force -Path $srcFfmpeg -Destination $destFfmpeg
    Copy-Item -Force -Path $srcFfprobe -Destination $destFfprobe

    $ffmpegSha = Get-FileSha256 $destFfmpeg
    $ffprobeSha = Get-FileSha256 $destFfprobe

    if ($ffmpegSha -ne $spec.ffmpeg_sha256.ToLowerInvariant()) {
      Fail ("ffmpeg sha256 mismatch for " + $target + " expected=" + $spec.ffmpeg_sha256 + " actual=" + $ffmpegSha)
    }
    if ($ffprobeSha -ne $spec.ffprobe_sha256.ToLowerInvariant()) {
      Fail ("ffprobe sha256 mismatch for " + $target + " expected=" + $spec.ffprobe_sha256 + " actual=" + $ffprobeSha)
    }

    Info ("PASS: " + $target + " -> " + $destDir)
  }

  Write-Host "DONE" -ForegroundColor Green
  exit 0
} finally {
  Remove-Item -Recurse -Force $workRoot -ErrorAction SilentlyContinue
}
