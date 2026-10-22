param([switch]$NoStart,[string]$TestRoot)
$ErrorActionPreference='Stop'
if(-not [Environment]::Is64BitOperatingSystem){throw 'Windows x64 required'}
$webview=@('HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}','HKCU:\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}')
if(-not @($webview|Where-Object{(Get-ItemProperty -LiteralPath $_ -Name pv -ErrorAction SilentlyContinue).pv}).Count){
  Write-Warning 'WebView2 was not detected. If the app cannot open, install Microsoft Edge WebView2 Runtime from https://developer.microsoft.com/microsoft-edge/webview2/'
}
$destination=if($TestRoot){Join-Path $TestRoot 'AIStatusLightBeta'}else{Join-Path $env:LOCALAPPDATA 'AIStatusLightBeta'}
$profileRoot=if($TestRoot){Join-Path $TestRoot 'profile'}else{$env:USERPROFILE}
New-Item -ItemType Directory -Force -Path $destination|Out-Null
$exe=Join-Path $destination 'AI-Status-Light.exe'
Get-CimInstance Win32_Process -Filter "Name='AI-Status-Light.exe'" | Where-Object ExecutablePath -eq $exe | ForEach-Object{$old=Get-Process -Id $_.ProcessId -ErrorAction SilentlyContinue;if($old){Stop-Process -Id $old.Id;$old.WaitForExit()}}
foreach($name in @('Start-Coze-Monitoring.ps1','Start-Coze-Monitoring.cmd','Start-Doubao-Monitoring.ps1','Start-Doubao-Monitoring.cmd','Start-Trae-Monitoring.ps1','Start-Trae-Monitoring.cmd')){Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $destination -Force}
foreach($name in @('AI-Status-Light.exe','status_writer.ps1','Configure-Hooks.ps1','Uninstall.ps1','Uninstall.cmd','Check.ps1','Check.cmd','Check-Doubao-Signal.ps1','Check-Doubao-Signal.cmd','doubao-status.js','doubao-status.ps1','trae-status.js','trae-status.ps1','coze-status.js','coze-status.ps1','README.txt','LICENSE','THIRD-PARTY.txt','SHA256.txt')){Copy-Item -LiteralPath (Join-Path $PSScriptRoot $name) -Destination $destination -Force}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'LICENSES') -Destination $destination -Recurse -Force
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'SOURCES') -Destination $destination -Recurse -Force
foreach($client in @('codex','workbuddy')){
  if(Test-Path -LiteralPath (Join-Path $profileRoot ".$client")){
    $leaf=if($client -eq 'codex'){'hooks.json'}else{'settings.json'}
    & (Join-Path $destination 'Configure-Hooks.ps1') -Client $client -ConfigPath (Join-Path (Join-Path $profileRoot ".$client") $leaf)
  }
  else{Write-Output "$client not detected. Install and open it, then run Install.cmd again."}
}

# Resolve a client exe: running process, then known paths, then registry, then manual input.
function Resolve-ClientExe{
  param([string]$ProcessName,[string[]]$KnownPaths,[string[]]$RegistryPattern,[string]$Label)
  $fromProcess=(Get-Process -Name $ProcessName -ErrorAction SilentlyContinue|Where-Object{$_.Path}|Select-Object -First 1).Path
  if($fromProcess){return $fromProcess}
  foreach($path in $KnownPaths){if(Test-Path -LiteralPath $path){return $path}}
  $roots=@('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall','HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall','HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall')
  foreach($root in $roots){
    foreach($key in @(Get-ChildItem $root -ErrorAction SilentlyContinue)){
      $p=Get-ItemProperty $key.PSPath -ErrorAction SilentlyContinue
      $matched=$false;foreach($pattern in $RegistryPattern){if($p.DisplayName -like $pattern){$matched=$true}};if($matched){
        if($p.DisplayIcon){
          $icon=($p.DisplayIcon -replace ',\d+$','').Trim('"')
          if($icon -and (Test-Path -LiteralPath $icon)){return $icon}
        }
        if($p.InstallLocation){
          foreach($leafName in @((Split-Path -Leaf $ProcessName)+'.exe')){
            $candidate=Join-Path $p.InstallLocation $leafName
            if(Test-Path -LiteralPath $candidate){return $candidate}
          }
        }
      }
    }
  }
  # Start Menu shortcuts record the real install path even for custom locations.
  $sh=New-Object -ComObject WScript.Shell
  $menus=@((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'),'C:\ProgramData\Microsoft\Windows\Start Menu\Programs',[Environment]::GetFolderPath('Desktop'),'C:\Users\Public\Desktop')
  foreach($menu in $menus){
    foreach($lnk in @(Get-ChildItem $menu -Filter '*.lnk' -ErrorAction SilentlyContinue)){
      $nameHit=$false
      foreach($pattern in $RegistryPattern){if($lnk.BaseName -like $pattern){$nameHit=$true}}
      if($lnk.BaseName -like '*状态灯*'){continue}
      if(!$nameHit){continue}
      try{
        $shortcut=$sh.CreateShortcut($lnk.FullName)
        $target=$shortcut.TargetPath
        if($target -and $target -match '\.exe$' -and $target -notmatch 'powershell' -and (Test-Path -LiteralPath $target)){return $target}
      }catch{}
    }
  }
  Write-Output "$Label was not detected automatically. If it is installed, paste the full path of its exe and press Enter; press Enter alone to skip $Label."
  $manual=Read-Host 'Path'
  if($manual){
    $manual=$manual.Trim().Trim('"')
    if(Test-Path -LiteralPath $manual){return $manual}
    Write-Output 'Path not found; skipping.'
  }
  return $null
}

$shortcutDefs=@(
  @{Label='扣子（AI 状态灯）'; Description='启动扣子并连接 AI 状态灯'; Script='Start-Coze-Monitoring.ps1'},
  @{Label='豆包（AI 状态灯）'; Description='启动豆包并连接 AI 状态灯'; Script='Start-Doubao-Monitoring.ps1'},
  @{Label='TRAE（AI 状态灯）'; Description='启动 TRAE 并连接 AI 状态灯'; Script='Start-Trae-Monitoring.ps1'}
)

if(-not $TestRoot){
$savedPaths=@{}
$shortcut=(New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'AI Status Light Beta.lnk'))
$shortcut.TargetPath=$exe;$shortcut.WorkingDirectory=$destination;$shortcut.Save()
$cozeVersions=Join-Path $env:LOCALAPPDATA 'Programs\Coze\versions'
$cozeVersion=Get-ChildItem -LiteralPath $cozeVersions -Directory -ErrorAction SilentlyContinue|Where-Object{Test-Path -LiteralPath (Join-Path $_.FullName 'Coze.exe')}|Sort-Object LastWriteTime -Descending|Select-Object -First 1
foreach($def in $shortcutDefs){
  $clientExe=$null
  if($def.Script -like '*Coze*'){
    if($cozeVersion){$clientExe=Join-Path $cozeVersion.FullName 'Coze.exe'}
  } elseif($def.Script -like '*Doubao*'){
    $clientExe=Resolve-ClientExe -ProcessName 'Doubao' -KnownPaths @((Join-Path $env:LOCALAPPDATA 'Doubao\Application\Doubao.exe'),(Join-Path $env:LOCALAPPDATA 'Doubao\Application\app\Doubao.exe')) -RegistryPattern @('*豆包*','*Doubao*') -Label 'Doubao'
  } else {
    $clientExe=Resolve-ClientExe -ProcessName 'TRAE SOLO CN' -KnownPaths @((Join-Path $env:LOCALAPPDATA 'Programs\TRAE SOLO CN\TRAE SOLO CN.exe')) -RegistryPattern @('*TRAE*') -Label 'TRAE'
  }
  if($clientExe){
    $key=if($def.Script -like '*Coze*'){'coze'}elseif($def.Script -like '*Doubao*'){'doubao'}else{'trae'}
    $savedPaths[$key]=$clientExe
    $clientShortcut=(New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) ($def.Label+'.lnk')))
    $clientShortcut.TargetPath=(Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe')
    $clientShortcut.Arguments='-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+(Join-Path $destination $def.Script)+'"'
    $clientShortcut.WorkingDirectory=$destination
    $clientShortcut.IconLocation=$clientExe+',0'
    $clientShortcut.Description=$def.Description
    $clientShortcut.Save()
  } else {
    Write-Output ($def.Label + ': client not found; shortcut skipped. Install it, then run Install.cmd again.')
  }
}
($savedPaths | ConvertTo-Json) | Set-Content -LiteralPath (Join-Path $destination 'client-paths.json') -Encoding UTF8
}
if(-not $NoStart -and -not $TestRoot){Start-Process -FilePath $exe -ArgumentList '--enable-autostart' -WindowStyle Normal}
Write-Output 'Installed. Launch Coze / Doubao / TRAE from their (AI status light) desktop shortcuts so the lamp can see them.'
