param([Parameter(Mandatory=$true)][string]$Zip)
$ErrorActionPreference='Stop'
$root=Join-Path ([IO.Path]::GetTempPath()) ('light-package-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root|Out-Null
Expand-Archive -LiteralPath $Zip -DestinationPath (Join-Path $root 'unpacked')
$package=(Get-ChildItem (Join-Path $root 'unpacked') -Directory|Select-Object -First 1).FullName
$profile=Join-Path $root 'profile'
foreach($client in @('codex','workbuddy')){New-Item -ItemType Directory -Force -Path (Join-Path $profile ".$client")|Out-Null}
$buddy=Join-Path $profile '.workbuddy\settings.json'
[IO.File]::WriteAllText($buddy,'{"custom":42,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo keep"}]}]}}')
& (Join-Path $package 'Install.ps1') -TestRoot $root -NoStart|Out-Null
& (Join-Path $package 'Install.ps1') -TestRoot $root -NoStart|Out-Null
$install=Join-Path $root 'AIStatusLightBeta'
$expected=(Get-Content (Join-Path $package 'SHA256.txt')).Split(' ')[0]
if((Get-FileHash (Join-Path $install 'AI-Status-Light.exe')).Hash -ne $expected){throw 'Installed executable checksum mismatch'}
foreach($file in @('Configure-Hooks.ps1','Uninstall.cmd','Check.cmd','LICENSE','THIRD-PARTY.txt','status_writer.ps1','Start-Coze-Monitoring.ps1','Start-Coze-Monitoring.cmd')){if(-not(Test-Path (Join-Path $install $file))){throw "Missing $file"}}
$settings=Get-Content $buddy -Raw|ConvertFrom-Json
if($settings.custom -ne 42 -or @($settings.hooks.Stop).Count -ne 2){throw 'Reinstall corrupts settings'}
& (Join-Path $install 'Uninstall.ps1') -TestRoot $root|Out-Null
$settings=Get-Content $buddy -Raw|ConvertFrom-Json
if($settings.custom -ne 42 -or @($settings.hooks.Stop).Count -ne 1){throw 'Uninstall corrupts settings'}
if((Get-ChildItem $package -Recurse -File | Where-Object Name -in @('status-monitor.ps1','credentials.json','snapshot.json','data.db','db.sqlite')).Count){throw 'Forbidden source/user data in package'}
Write-Output 'PASS: extracted package installs/reinstalls/uninstalls safely in an isolated profile; executable hash matches; no legacy monitor or user database shipped.'
Write-Output "Artifacts: $root"
