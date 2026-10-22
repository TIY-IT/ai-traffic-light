$ErrorActionPreference='Stop'
$root=Join-Path ([IO.Path]::GetTempPath()) ('light-delivery-test-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root|Out-Null
$config=Join-Path $root 'settings.json'
$writer=Join-Path (Get-Location) 'hooks\status_writer.ps1'
[IO.File]::WriteAllText($config,'{"extra":"keep","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo preserve-me"}]}]}}')
foreach($client in @('codex','workbuddy')){
  & .\delivery\Configure-Hooks.ps1 -Client $client -ConfigPath $config -WriterPath $writer|Out-Null
  $once=Get-Content $config -Raw|ConvertFrom-Json
  & .\delivery\Configure-Hooks.ps1 -Client $client -ConfigPath $config -WriterPath $writer|Out-Null
  $twice=Get-Content $config -Raw|ConvertFrom-Json
  if($twice.extra -ne 'keep' -or @($twice.hooks.Stop).Count -ne 2){throw 'Merge or idempotency failed'}
  if(@($twice.hooks.Stop[0].hooks)[0].command -ne 'echo preserve-me'){throw 'Existing hook changed'}
  & .\delivery\Configure-Hooks.ps1 -Client $client -ConfigPath $config -Uninstall|Out-Null
  $removed=Get-Content $config -Raw|ConvertFrom-Json
  if(@($removed.hooks.Stop).Count -ne 1){throw 'Uninstall removed other handlers'}
}
$signals=Join-Path $root 'signals'
foreach($state in @('working','completed','waiting','error')){
  '{"session_id":"test-session","cwd":"C:/fixture"}' | powershell.exe -NoProfile -ExecutionPolicy Bypass -File $writer -Client codex -State $state -StateDirectory $signals -EmitEmptyJson|Out-Null
  if($LASTEXITCODE -ne 0){throw 'Writer failed'}
  $value=Get-Content (Join-Path $signals 'codex-test-session.json') -Raw|ConvertFrom-Json
  if($value.state -ne $state){throw "Wrong state: $state"}
}
'{}' | powershell.exe -NoProfile -ExecutionPolicy Bypass -File $writer -Client workbuddy -State working -StateDirectory $signals -EmitEmptyJson|Out-Null
if(@(Get-ChildItem $signals).Count -ne 1){throw 'Missing session id created phantom state'}
Write-Output 'PASS: merge, repeat install, uninstall preservation, four signal states, missing-ID rejection.'
Write-Output "Isolated test artifacts: $root"
