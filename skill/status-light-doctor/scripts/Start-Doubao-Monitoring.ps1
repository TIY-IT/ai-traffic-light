# Doubao launcher used by the desktop shortcut. Debugging stays loopback-only.
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

$doubaoExe = (Get-Process -Name Doubao -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1).Path
if (!$doubaoExe) { $doubaoExe = Join-Path $env:LOCALAPPDATA 'Doubao\Application\Doubao.exe' }
# Install.cmd remembers the path it resolved (custom installs live outside the standard folder).
$pathsFile = Join-Path $installRoot 'client-paths.json'
if (!(Test-Path -LiteralPath $doubaoExe) -and (Test-Path -LiteralPath $pathsFile)) {
  try {
    $saved = (Get-Content -LiteralPath $pathsFile -Raw | ConvertFrom-Json).doubao
    if ($saved -and (Test-Path -LiteralPath $saved)) { $doubaoExe = $saved }
  } catch { }
}
if (!(Test-Path -LiteralPath $doubaoExe)) {
  Add-Type -AssemblyName PresentationFramework
  [System.Windows.MessageBox]::Show('未找到豆包。请先安装豆包；若它安装在自定义位置，请重新运行 Install.cmd，并在提示时输入豆包的 Doubao.exe 完整路径。','豆包 AI 状态灯') | Out-Null
  exit 1
}

$port = 19224
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
if ($listeners.Count) {
  if (@($listeners | Where-Object { $_.LocalAddress -ne '127.0.0.1' }).Count) { throw 'Port 19224 is not loopback-only; refusing to connect.' }
  $owner = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
  if ($owner.ProcessName -ne 'Doubao') { throw 'Port 19224 belongs to another application.' }
  Start-Process -FilePath $doubaoExe
  exit
}
$running = @(Get-Process -Name Doubao -ErrorAction SilentlyContinue)
if ($running.Count) {
  Add-Type -AssemblyName PresentationFramework
  $choice = [System.Windows.MessageBox]::Show(
    "豆包当前是普通模式，状态灯无法识别。`n`n是否立即重启豆包并启用状态灯？`n正在生成的回复可能会中断。",
    '豆包 AI 状态灯',
    [System.Windows.MessageBoxButton]::YesNo,
    [System.Windows.MessageBoxImage]::Question
  )
  if ($choice -ne [System.Windows.MessageBoxResult]::Yes) {
    Start-Process -FilePath $doubaoExe
    exit
  }
  $running | Stop-Process
  $deadline = [DateTime]::UtcNow.AddSeconds(10)
  while ((Get-Process -Name Doubao -ErrorAction SilentlyContinue) -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 100
  }
  if (Get-Process -Name Doubao -ErrorAction SilentlyContinue) {
    throw 'Doubao did not exit completely. Exit it from the tray and try again.'
  }
}
Start-Process -FilePath $doubaoExe -ArgumentList '--registry-web-browser','--remote-debugging-port=19224' -WindowStyle Hidden
