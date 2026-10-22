$ErrorActionPreference='Continue'
Write-Output ('Windows x64: '+[Environment]::Is64BitOperatingSystem)
foreach($name in @('Codex','WorkBuddy','Marvis','ZCode')){Write-Output ($name+' running: '+[bool](Get-Process $name -ErrorAction SilentlyContinue))}
foreach($client in @('codex','workbuddy')){
  $leaf=if($client -eq 'codex'){'hooks.json'}else{'settings.json'}
  $path=Join-Path (Join-Path $env:USERPROFILE ".$client") $leaf
  $configured=$false
  if(Test-Path -LiteralPath $path){
    try{$value=Get-Content -LiteralPath $path -Raw -Encoding UTF8|ConvertFrom-Json
      foreach($event in $value.hooks.PSObject.Properties){foreach($group in $event.Value){foreach($hook in $group.hooks){
        if($hook.command -match '-EncodedCommand\s+([A-Za-z0-9+/=]+)'){
          if([Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($Matches[1])).StartsWith('# AIStatusLight.Beta1')){$configured=$true}
        }
      }}}
    }catch{}
  }
  Write-Output "$client hooks registered: $configured (Codex trust must be checked inside Codex)"
}
Write-Output ('Recent signal files: '+@(Get-ChildItem (Join-Path $env:USERPROFILE '.ai-traffic-light\sessions') -Filter '*.json' -ErrorAction SilentlyContinue).Count)
Write-Output 'This check does not print chat contents or credentials.'
