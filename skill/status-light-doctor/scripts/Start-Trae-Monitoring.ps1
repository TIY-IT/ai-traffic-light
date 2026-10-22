# TRAE launcher used by the desktop shortcut. Debugging stays loopback-only.
$ErrorActionPreference = 'Stop'
$installRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
# Prefer the recovery build when it sits next to this script; the delivery keeps the plain name only.
$recoveryLight = Join-Path $installRoot 'AI-Status-Light-Recovery.exe'
if (Test-Path -LiteralPath $recoveryLight) { $statusLight = $recoveryLight } else { $statusLight = Join-Path $installRoot 'AI-Status-Light.exe' }
# Match the running light by its actual file name, so the recovery layout deduplicates too.
$lightName = Split-Path -Leaf $statusLight
$statusLightProcess = Get-CimInstance Win32_Process -Filter "Name='$lightName'" -ErrorAction SilentlyContinue |
  Where-Object { $_.ExecutablePath -eq $statusLight }
if ((Test-Path -LiteralPath $statusLight) -and !$statusLightProcess) {
  Start-Process -FilePath $statusLight -WindowStyle Normal
}

$traeExe = (Get-Process -Name 'TRAE SOLO CN' -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1).Path
if (!$traeExe) { $traeExe = Join-Path $env:LOCALAPPDATA 'Programs\TRAE SOLO CN\TRAE SOLO CN.exe' }
# Install.cmd remembers the path it resolved (custom installs live outside the standard folder).
$pathsFile = Join-Path $installRoot 'client-paths.json'
if (!(Test-Path -LiteralPath $traeExe) -and (Test-Path -LiteralPath $pathsFile)) {
  try {
    $saved = (Get-Content -LiteralPath $pathsFile -Raw | ConvertFrom-Json).trae
    if ($saved -and (Test-Path -LiteralPath $saved)) { $traeExe = $saved }
  } catch { }
}
if (!(Test-Path -LiteralPath $traeExe)) {
  Add-Type -AssemblyName PresentationFramework
  [System.Windows.MessageBox]::Show('未找到 TRAE。请先安装 TRAE；若它安装在自定义位置，请重新运行 Install.cmd，并在提示时输入 TRAE 的完整路径。','TRAE AI 状态灯') | Out-Null
  exit 1
}

$port = 19226
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
if ($listeners.Count) {
  if (@($listeners | Where-Object { $_.LocalAddress -ne '127.0.0.1' }).Count) { throw 'Port 19226 is not loopback-only; refusing to connect.' }
  $owner = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
  if ($owner.ProcessName -ne 'TRAE SOLO CN') { throw 'Port 19226 belongs to another application.' }
  Start-Process -FilePath $traeExe
  exit
}
$running = @(Get-Process -Name 'TRAE SOLO CN' -ErrorAction SilentlyContinue)
if ($running.Count) {
  Add-Type -AssemblyName PresentationFramework
  $choice = [System.Windows.MessageBox]::Show(
    "TRAE 当前是普通模式，状态灯无法识别。`n`n是否立即重启 TRAE 并启用状态灯？`n正在生成的任务可能会中断。",
    'TRAE AI 状态灯',
    [System.Windows.MessageBoxButton]::YesNo,
    [System.Windows.MessageBoxImage]::Question
  )
  if ($choice -ne [System.Windows.MessageBoxResult]::Yes) {
    Start-Process -FilePath $traeExe
    exit
  }
  $running | Stop-Process
  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  while ((Get-Process -Name 'TRAE SOLO CN' -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Get-Process -Name 'TRAE SOLO CN' -ErrorAction SilentlyContinue) {
    throw 'TRAE did not exit completely. Exit it from the tray and try again.'
  }
}
Start-Process -FilePath $traeExe -ArgumentList '--remote-debugging-port=19226' -WindowStyle Hidden
