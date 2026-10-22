# Coze launcher used by the desktop shortcut. Debugging stays loopback-only.
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

$versions = Join-Path $env:LOCALAPPDATA 'Programs\Coze\versions'
$version = Get-ChildItem -LiteralPath $versions -Directory -ErrorAction SilentlyContinue |
  Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'Coze.exe') } |
  Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (!$version) { throw 'Coze desktop installation not found.' }
$cozeExe = Join-Path $version.FullName 'Coze.exe'

$port = 19222
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
if ($listeners.Count) {
  if (@($listeners | Where-Object { $_.LocalAddress -ne '127.0.0.1' }).Count) { throw 'Port 19222 is not loopback-only; refusing to connect.' }
  $owner = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
  if ($owner.ProcessName -ne 'Coze') { throw 'Port 19222 belongs to another application.' }
  Start-Process -FilePath $cozeExe -ArgumentList '--app_running'
  exit
}
$running = @(Get-Process -Name Coze -ErrorAction SilentlyContinue)
if ($running.Count) {
  Add-Type -AssemblyName PresentationFramework
  $choice = [System.Windows.MessageBox]::Show(
    "扣子当前是普通模式，状态灯无法识别。`n`n是否立即重启扣子并启用状态灯？`n正在生成的任务可能会中断。",
    '扣子 AI 状态灯',
    [System.Windows.MessageBoxButton]::YesNo,
    [System.Windows.MessageBoxImage]::Question
  )
  if ($choice -ne [System.Windows.MessageBoxResult]::Yes) {
    Start-Process -FilePath $cozeExe -ArgumentList '--app_running'
    exit
  }
  $running | Stop-Process
  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  while ((Get-Process -Name Coze -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Get-Process -Name Coze -ErrorAction SilentlyContinue) {
    throw 'Coze did not exit completely. Exit it from the tray and try again.'
  }
}
Start-Process -FilePath $cozeExe -ArgumentList '--remote-debugging-address=127.0.0.1','--remote-debugging-port=19222','--app_running' -WindowStyle Hidden
