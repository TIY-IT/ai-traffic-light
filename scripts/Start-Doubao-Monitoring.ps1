# Doubao launcher used by the desktop shortcut. Debugging stays loopback-only.
$ErrorActionPreference = 'Stop'
$installRoot = Split-Path -Parent $MyInvocation.MyCommand.Path
Add-Type -AssemblyName PresentationFramework

function Show-Error {
  param([string]$Text)
  [System.Windows.MessageBox]::Show($Text, '豆包 AI 状态灯', [System.Windows.MessageBoxButton]::OK, [System.Windows.MessageBoxImage]::Warning) | Out-Null
}

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

function Resolve-DoubaoExe {
  $fromProcess = (Get-Process -Name Doubao -ErrorAction SilentlyContinue | Where-Object { $_.Path } | Select-Object -First 1).Path
  if ($fromProcess) { return $fromProcess }
  $known = @(
    (Join-Path $env:LOCALAPPDATA 'Doubao\Application\Doubao.exe'),
    (Join-Path $env:LOCALAPPDATA 'Doubao\Application\app\Doubao.exe')
  )
  foreach ($p in $known) { if (Test-Path -LiteralPath $p) { return $p } }
  # client-paths.json remembers a path the user entered before.
  $pathsFile = Join-Path $installRoot 'client-paths.json'
  if (Test-Path -LiteralPath $pathsFile) {
    try {
      $saved = (Get-Content -LiteralPath $pathsFile -Raw | ConvertFrom-Json).doubao
      if ($saved -and (Test-Path -LiteralPath $saved)) { return $saved }
    } catch { }
  }
  # Start Menu + Desktop shortcut scan (finds custom install locations).
  $sh = New-Object -ComObject WScript.Shell
  $menus = @((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'), 'C:\ProgramData\Microsoft\Windows\Start Menu\Programs', [Environment]::GetFolderPath('Desktop'), 'C:\Users\Public\Desktop')
  foreach ($menu in $menus) {
    foreach ($lnk in @(Get-ChildItem $menu -Filter '*.lnk' -ErrorAction SilentlyContinue)) {
      $nameHit = $false
      foreach ($pat in @('*豆包*','*Doubao*')) { if ($lnk.BaseName -like $pat) { $nameHit = $true } }
      if ($lnk.BaseName -like '*状态灯*') { continue }
      if (!$nameHit) { continue }
      try {
        $shortcut = $sh.CreateShortcut($lnk.FullName)
        $target = $shortcut.TargetPath
        if ($target -and $target -match '\.exe$' -and $target -notmatch 'powershell' -and (Test-Path -LiteralPath $target)) { return $target }
      } catch { }
    }
  }
  return $null
}

$doubaoExe = Resolve-DoubaoExe
if (!$doubaoExe) {
  Add-Type -AssemblyName PresentationFramework
  Add-Type -AssemblyName Microsoft.VisualBasic
  $pasted = [Microsoft.VisualBasic.Interaction]::InputBox(
    '未检测到豆包。若已安装,请输入豆包 Doubao.exe 的完整路径后点确定;留空则跳过豆包。(输入一次会被记住)',
    '豆包 AI 状态灯', '')
  Add-Type -AssemblyName Microsoft.VisualBasic
  if ($pasted) {
    $pasted = $pasted.Trim().Trim('"')
    if (Test-Path -LiteralPath $pasted) {
      $doubaoExe = $pasted
      try {
        $pathsFile = Join-Path $installRoot 'client-paths.json'
        $saved = if (Test-Path -LiteralPath $pathsFile) { Get-Content -LiteralPath $pathsFile -Raw | ConvertFrom-Json } else { New-Object psobject }
        if ($saved.PSObject.Properties['doubao']) { $saved.doubao = $doubaoExe } else { $saved | Add-Member -NotePropertyName doubao -NotePropertyValue $doubaoExe }
        $saved | ConvertTo-Json | Set-Content -LiteralPath $pathsFile -Encoding UTF8
      } catch { }
    } else {
      Show-Error '输入的路径不存在。跳过豆包。'
      exit 1
    }
  } else {
    exit 1
  }
}

$port = 19224
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $port -ErrorAction SilentlyContinue)
if ($listeners.Count) {
  if (@($listeners | Where-Object { $_.LocalAddress -ne '127.0.0.1' }).Count) { Show-Error '端口 19224 不是仅本机回环,拒绝连接。'; exit 1 }
  $owner = Get-Process -Id $listeners[0].OwningProcess -ErrorAction Stop
  if ($owner.ProcessName -ne 'Doubao') { Show-Error '端口 19224 被其他程序占用。'; exit 1 }
  Start-Process -FilePath $doubaoExe
  exit
}
$running = @(Get-Process -Name Doubao -ErrorAction SilentlyContinue)
if ($running.Count) {
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
    Show-Error '豆包未完全退出。请从托盘退出豆包后重试。'
    exit 1
  }
}
Start-Process -FilePath $doubaoExe -ArgumentList '--registry-web-browser','--remote-debugging-port=19224' -WindowStyle Hidden
