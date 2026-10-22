param([string]$TestRoot)
$ErrorActionPreference='Stop'
$destination=if($TestRoot){Join-Path $TestRoot 'AIStatusLightBeta'}else{Join-Path $env:LOCALAPPDATA 'AIStatusLightBeta'}
$profileRoot=if($TestRoot){Join-Path $TestRoot 'profile'}else{$env:USERPROFILE}
$exe=Join-Path $destination 'AI-Status-Light.exe'
Get-CimInstance Win32_Process -Filter "Name='AI-Status-Light.exe'" | Where-Object ExecutablePath -eq $exe | ForEach-Object{Stop-Process -Id $_.ProcessId}
foreach($client in @('codex','workbuddy')){
  $leaf=if($client -eq 'codex'){'hooks.json'}else{'settings.json'}
  $config=Join-Path (Join-Path $profileRoot ".$client") $leaf
  if(Test-Path -LiteralPath $config){& (Join-Path $PSScriptRoot 'Configure-Hooks.ps1') -Client $client -ConfigPath $config -Uninstall}
}
if(-not $TestRoot){
$run='HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$value=(Get-ItemProperty $run -Name 'AI Traffic Light' -ErrorAction SilentlyContinue).'AI Traffic Light'
if($value -and $value.Trim('"',' ') -eq $exe){Remove-ItemProperty $run -Name 'AI Traffic Light'}
$shortcut=Join-Path ([Environment]::GetFolderPath('Desktop')) 'AI Status Light Beta.lnk'
if(Test-Path -LiteralPath $shortcut){Remove-Item -LiteralPath $shortcut}
$cozeShortcut=Join-Path ([Environment]::GetFolderPath('Desktop')) '扣子（AI 状态灯）.lnk'
if(Test-Path -LiteralPath $cozeShortcut){Remove-Item -LiteralPath $cozeShortcut}
}
Write-Output "Hooks and startup removed. Program files retained at $destination for recovery; delete that folder manually if no longer needed. Session records were not deleted."
