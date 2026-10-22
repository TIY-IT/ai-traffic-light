$ErrorActionPreference='Stop'
$root=Split-Path -Parent $PSScriptRoot
$release=Join-Path $root ('releases\AI-Status-Light-Beta-'+(Get-Date -Format 'yyyyMMdd-HHmmss'))
New-Item -ItemType Directory -Path $release|Out-Null
# Build in-pipeline: without --features custom-protocol the exe is a dev-mode
# binary that loads devUrl at runtime (WebView shows "无法访问此页面").
& cargo build --release --manifest-path (Join-Path $root 'Cargo.toml') -p ai-traffic-light --features ai-traffic-light/custom-protocol
if($LASTEXITCODE -ne 0){throw 'build failed'}
Copy-Item -LiteralPath (Join-Path $root 'target\release\ai-traffic-light.exe') -Destination (Join-Path $release 'AI-Status-Light.exe')
foreach($name in @('Install.cmd','Install.ps1','Uninstall.cmd','Uninstall.ps1','Configure-Hooks.ps1','Check.cmd','Check.ps1','Check-Doubao-Signal.ps1','Check-Doubao-Signal.cmd','README.txt')){Copy-Item -LiteralPath (Join-Path $root "delivery\$name") -Destination $release}
# Ship the full source (excluding build artifacts) for on-site inspection and repair.
$sourceDir=Join-Path $release 'source'
New-Item -ItemType Directory -Force -Path $sourceDir|Out-Null
robocopy $root $sourceDir /E /XD target node_modules releases .git dist /XF *.zip *.exe | Out-Null
if($LASTEXITCODE -gt 7){throw 'source copy failed'}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Start-Coze-Monitoring.ps1'),(Join-Path $root 'delivery\Start-Coze-Monitoring.cmd'),(Join-Path $PSScriptRoot 'Start-Doubao-Monitoring.ps1'),(Join-Path $PSScriptRoot 'Start-Doubao-Monitoring.cmd'),(Join-Path $PSScriptRoot 'Start-Trae-Monitoring.ps1'),(Join-Path $PSScriptRoot 'Start-Trae-Monitoring.cmd') -Destination $release
Copy-Item -LiteralPath (Join-Path $root 'src-tauri\resources\doubao-status.js'),(Join-Path $root 'src-tauri\resources\doubao-status.ps1'),(Join-Path $root 'src-tauri\resources\trae-status.js'),(Join-Path $root 'src-tauri\resources\trae-status.ps1'),(Join-Path $root 'src-tauri\resources\coze-status.js'),(Join-Path $root 'src-tauri\resources\coze-status.ps1') -Destination $release
Copy-Item -LiteralPath (Join-Path $root 'hooks\status_writer.ps1'),(Join-Path $root 'LICENSE') -Destination $release
Copy-Item -LiteralPath (Join-Path $root 'TUTORIAL.md') -Destination $release
New-Item -ItemType Directory -Force -Path (Join-Path $release 'skill\status-light-doctor') | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'skill\status-light-doctor\SKILL.md') -Destination (Join-Path $release 'skill\status-light-doctor\SKILL.md')
Copy-Item -LiteralPath (Join-Path $root 'skill\status-light-doctor\scripts') -Destination (Join-Path $release 'skill\status-light-doctor\scripts') -Recurse -Force
& node (Join-Path $PSScriptRoot 'license-notices.cjs') $release
if($LASTEXITCODE -ne 0){throw 'Notice generation failed'}
$hash=(Get-FileHash (Join-Path $release 'AI-Status-Light.exe') -Algorithm SHA256).Hash
[IO.File]::WriteAllText((Join-Path $release 'SHA256.txt'),"$hash  AI-Status-Light.exe`r`n")
Get-ChildItem -LiteralPath $release -Recurse -File | ForEach-Object { $_.LastWriteTime=Get-Date }
Compress-Archive -LiteralPath $release -DestinationPath ($release+'.zip')
Write-Output ($release+'.zip')
