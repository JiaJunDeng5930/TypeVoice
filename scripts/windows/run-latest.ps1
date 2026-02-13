param(
  [string]$RepoRoot = "",
  [string]$LogDir = ""
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Fail([string]$Message) {
  Write-Host "FAIL: $Message" -ForegroundColor Red
  exit 1
}

function Info([string]$Message) {
  Write-Host "INFO: $Message" -ForegroundColor Cyan
}

function Resolve-RepoRoot([string]$InputRoot) {
  if ($InputRoot -and (Test-Path $InputRoot)) {
    return (Resolve-Path $InputRoot).Path
  }
  return (Resolve-Path (Join-Path $PSScriptRoot "..\..\")).Path
}

function Kill-StaleProcesses([string]$RootNeedle) {
  Info "stopping stale TypeVoice Windows processes"
  try {
    $desktopProcesses = Get-CimInstance Win32_Process -Filter "Name='typevoice-desktop.exe'"
    foreach ($proc in $desktopProcesses) {
      Write-Host "killing typevoice-desktop.exe: pid=$($proc.ProcessId)"
      Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
    }

    $nodeProcesses = Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object {
      $_.CommandLine -and $_.CommandLine.ToLowerInvariant().Contains($RootNeedle)
    }
    foreach ($proc in $nodeProcesses) {
      Write-Host "killing node.exe from workspace: pid=$($proc.ProcessId)"
      Stop-Process -Id $proc.ProcessId -Force -ErrorAction SilentlyContinue
    }
  } catch {
    # best effort cleanup
  }

  Start-Sleep -Seconds 1
  $stillAlive = Get-Process -Name typevoice-desktop -ErrorAction SilentlyContinue
  if ($stillAlive) {
    Fail "failed to stop stale typevoice-desktop.exe"
  }
}

function Wait-ForFrontendBinary {
  param(
    [string]$ExePath,
    [int]$TimeoutSeconds = 180
  )
  $target = $ExePath.ToLowerInvariant()
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)

  while ((Get-Date) -lt $deadline) {
    $proc = Get-CimInstance Win32_Process -Filter "Name='typevoice-desktop.exe'" | Where-Object {
      ($_.ExecutablePath -and $_.ExecutablePath.ToLowerInvariant() -eq $target) -or
      ($_.CommandLine -and $_.CommandLine.ToLowerInvariant().Contains($target))
    }
    if ($proc) {
      return $proc
    }
    Start-Sleep -Milliseconds 500
  }
  return $null
}

$repoRoot = Resolve-RepoRoot $RepoRoot
$repoRootLower = $repoRoot.ToLowerInvariant()
if ($repoRootLower -like "\\\\wsl.localhost\\*") {
  Fail "Refusing UNC path. Run this script from a real Windows path."
}
$desktopPath = Join-Path $repoRoot "apps\desktop"
$logDir = if ($LogDir) { $LogDir } else { Join-Path $repoRoot "tmp\typevoice-logs" }
$logFile = Join-Path $logDir "tauri-latest-run.txt"
$frontendBinary = Join-Path $desktopPath "src-tauri\target\debug\typevoice-desktop.exe"
$desktopPkg = Join-Path $desktopPath "package.json"

if (-not (Test-Path $desktopPkg)) {
  Fail "expected repo root's apps\\desktop folder to contain package.json: $desktopPkg"
}

New-Item -ItemType Directory -Force -Path $logDir | Out-Null
Set-Content -Path $logFile -Value ("`r`n=== run-latest.ps1 " + (Get-Date).ToString("s") + " ===`r`n")

$rootNeedle = $repoRootLower

Kill-StaleProcesses -RootNeedle $rootNeedle

Info "building frontend bundle (npm run build)"
Set-Location $desktopPath
& npm run build 2>&1 | Tee-Object -FilePath $logFile -Append | Out-Host
if ($LASTEXITCODE -ne 0) {
  Fail "npm run build failed (exit code $LASTEXITCODE). See $logFile"
}

Info "starting tauri dev with logs redirected to $logFile"
$env:RUST_BACKTRACE = "1"
$env:RUST_LOG = "debug"
$desktopDirEscaped = $desktopPath.Replace('"', '""')
$logFileEscaped = $logFile.Replace('"', '""')
$devCommand = "cd /d `"$desktopDirEscaped`" && set RUST_BACKTRACE=1 && set RUST_LOG=debug && npm run tauri dev >> `"$logFileEscaped`" 2>&1"
$devProcess = Start-Process -FilePath "cmd.exe" -ArgumentList @("/d", "/c", $devCommand) -PassThru

Info "waiting for runtime process to appear: $frontendBinary"
$runtime = Wait-ForFrontendBinary -ExePath $frontendBinary
if (-not $runtime) {
  Write-Host "dev command pid=$($devProcess.Id)"
  Fail "typevoice-desktop.exe did not start. Check log: $logFile"
}

Info "runtime started successfully"
Write-Host "PID: $($runtime.ProcessId)"
Write-Host "Path: $($runtime.ExecutablePath)"
Write-Host "StartTime: $($runtime.CreationDate)"
Write-Host "Log: $logFile"
Write-Host "Last 30 log lines:"
Get-Content -Path $logFile -Tail 30 | ForEach-Object { Write-Host $_ }
