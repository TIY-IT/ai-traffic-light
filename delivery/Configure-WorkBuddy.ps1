param([switch]$Uninstall, [string]$SettingsPath)
$ErrorActionPreference = 'Stop'
if (-not $SettingsPath) { $SettingsPath = Join-Path $env:USERPROFILE '.workbuddy\settings.json' }
if (-not (Test-Path -LiteralPath $SettingsPath)) { throw 'WorkBuddy settings not found. Install and launch WorkBuddy first.' }
$config = Get-Content -LiteralPath $SettingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
if (-not $config.PSObject.Properties['hooks']) { $config | Add-Member -NotePropertyName hooks -NotePropertyValue (New-Object psobject) }
$marker = '# AIStatusLight.WorkBuddy'
function Is-OurHandler($Handler) {
  $command = [string]$Handler.command
  if ($command -notmatch '-EncodedCommand\s+([A-Za-z0-9+/=]+)') { return $false }
  try { return [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($Matches[1])).StartsWith($marker) } catch { return $false }
}
# Remove only our own handlers, including when mixed with somebody else's hooks.
foreach ($property in @($config.hooks.PSObject.Properties)) {
  $groups = @()
  foreach ($group in @($property.Value)) {
    $remaining = @($group.hooks | Where-Object { -not (Is-OurHandler $_) })
    if ($remaining.Count) { $group.hooks = $remaining; $groups += $group }
  }
  $config.hooks.($property.Name) = $groups
}
if (-not $Uninstall) {
  $hook = (Join-Path $PSScriptRoot 'workbuddy-portable-hook.ps1').Replace("'", "''")
  foreach ($event in @('SessionStart','UserPromptSubmit','PermissionRequest','Stop','StopFailure','SessionEnd')) {
    $scriptText = $marker + "`n& '" + $hook + "' -Event '" + $event + "'"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($scriptText))
    $registration = [pscustomobject]@{ hooks = @([pscustomobject]@{ type = 'command'; command = "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand $encoded"; timeout = 5 }) }
    if ($config.hooks.PSObject.Properties[$event]) { $config.hooks.$event = @($config.hooks.$event) + @($registration) }
    else { $config.hooks | Add-Member -NotePropertyName $event -NotePropertyValue @($registration) }
  }
}
$backup = $SettingsPath + '.ai-light-' + (Get-Date -Format 'yyyyMMdd-HHmmssfff') + '.bak'
Copy-Item -LiteralPath $SettingsPath -Destination $backup
$temp = $SettingsPath + '.ai-light.tmp'
[IO.File]::WriteAllText($temp, ($config | ConvertTo-Json -Depth 100), (New-Object Text.UTF8Encoding($false)))
Move-Item -LiteralPath $temp -Destination $SettingsPath -Force
Write-Output "Saved. Backup: $backup"
Write-Output 'Restart WorkBuddy after its current tasks finish.'
