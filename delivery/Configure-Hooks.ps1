param([ValidateSet('codex','workbuddy')][string]$Client,[switch]$Uninstall,[string]$ConfigPath,[string]$WriterPath)
$ErrorActionPreference='Stop'
if (-not $ConfigPath) {
  $leaf=if($Client -eq 'codex'){'hooks.json'}else{'settings.json'}
  $ConfigPath=Join-Path (Join-Path $env:USERPROFILE ".$Client") $leaf
}
if (-not $WriterPath) {$WriterPath=Join-Path $PSScriptRoot 'status_writer.ps1'}
$config=if(Test-Path -LiteralPath $ConfigPath){Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8 | ConvertFrom-Json}else{[pscustomobject]@{}}
if(-not $config.PSObject.Properties['hooks']){$config|Add-Member hooks ([pscustomobject]@{})}
$marker='# AIStatusLight.Beta1'
foreach($property in @($config.hooks.PSObject.Properties)){
  $groups=@()
  foreach($group in @($property.Value)){
    $kept=@($group.hooks|Where-Object{
      $ours=$false
      if($_.command -match '-EncodedCommand\s+([A-Za-z0-9+/=]+)'){
        try{$ours=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($Matches[1])).StartsWith($marker)}catch{}
      }
      if($_.command -like '*status_writer.ps1*'){$ours=$true}
      -not $ours
    })
    if($kept.Count){$group.hooks=$kept;$groups+=@($group)}
  }
  $config.hooks.($property.Name)=$groups
}
if(-not $Uninstall){
  if(-not(Test-Path -LiteralPath $WriterPath)){throw 'Hook writer missing'}
  $events=[ordered]@{SessionStart='idle';UserPromptSubmit='working';PreToolUse='working';PostToolUse='working';PermissionRequest='waiting';Stop='completed';SessionEnd='idle'}
  if($Client -eq 'codex'){$events['Interrupt']='completed'}else{$events['StopFailure']='error'}
  foreach($event in $events.Keys){
    if($Client -eq 'workbuddy'){
      # WorkBuddy(0908 实测)不执行 -EncodedCommand 形式的处理器,普通命令形式才触发。
      $command="powershell.exe -NoProfile -ExecutionPolicy Bypass -File `"$WriterPath`" -Client '$Client' -State '"+$events[$event]+"' -EmitEmptyJson"
    } else {
      $script=$marker+"`n& '"+$WriterPath.Replace("'","''")+"' -Client '$Client' -State '"+$events[$event]+"' -EmitEmptyJson"
      $encoded=[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($script))
      # No timeout field: WorkBuddy applies its proven 60s default; a small custom value risks killing PowerShell on cold start.
      $command="powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded"
    }
    $entry=[pscustomobject]@{hooks=@([pscustomobject]@{type='command';command=$command})}
    if(-not $config.hooks.PSObject.Properties[$event]){$config.hooks|Add-Member $event @()}
    $config.hooks.$event=@($config.hooks.$event)+@($entry)
  }
}
New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ConfigPath)|Out-Null
if(Test-Path -LiteralPath $ConfigPath){Copy-Item -LiteralPath $ConfigPath -Destination ($ConfigPath+'.ai-light-'+(Get-Date -Format 'yyyyMMddHHmmssfff')+'.bak')}
$temp=$ConfigPath+'.ai-light.tmp'
[IO.File]::WriteAllText($temp,($config|ConvertTo-Json -Depth 100),(New-Object Text.UTF8Encoding($false)))
Move-Item -LiteralPath $temp -Destination $ConfigPath -Force
Write-Output "$Client configuration saved. Review/trust hooks inside Codex; restart WorkBuddy after current tasks finish."
