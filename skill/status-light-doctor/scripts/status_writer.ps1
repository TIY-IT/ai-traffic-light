param(
    [ValidateSet("codebuddy", "codex", "claude", "workbuddy")]
    [string]$Client = "codebuddy",
    [Parameter(Mandatory = $true)]
    [ValidateSet("idle", "working", "waiting", "completed", "error")]
    [string]$State,
    [string]$Message = "",
    [switch]$NotificationOnly,
    [switch]$EmitEmptyJson,
    [string]$StateDirectory
)

$utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
try {
    [Console]::InputEncoding = $utf8WithoutBom
    [Console]::OutputEncoding = $utf8WithoutBom
} catch {
    # Some hosts do not expose a mutable console; file output still uses UTF-8 below.
}
$OutputEncoding = $utf8WithoutBom
# Never block on stdin: hosts may keep the pipe open (WorkBuddy does). Read with a hard 800ms bound.
$payloadText = ''
try {
    if ([Console]::IsInputRedirected) {
        $task = [Console]::In.ReadToEndAsync()
        if ($task.Wait(800)) { $payloadText = $task.Result }
    }
} catch { }
$sessionsDir = Join-Path ([Environment]::GetFolderPath("UserProfile")) ".ai-traffic-light\sessions"
if ($StateDirectory) { $sessionsDir = $StateDirectory }
$textWaitingPermission = [regex]::Unescape("\u7b49\u5f85\u6743\u9650\u786e\u8ba4")
$textWaitingInput = [regex]::Unescape("\u7b49\u5f85\u8865\u5145\u4fe1\u606f")
$textCompleted = [regex]::Unescape("\u56de\u590d\u5b8c\u6210")
$textWaitingChoice = [regex]::Unescape("\u7b49\u5f85\u9009\u62e9")
$textToolFailed = [regex]::Unescape("\u5de5\u5177\u6267\u884c\u5931\u8d25")
$textPermissionDenied = [regex]::Unescape("\u6743\u9650\u88ab\u62d2\u7edd")
$textWaitingChoiceOrConfirmation = [regex]::Unescape("\u7b49\u5f85\u9009\u62e9\u6216\u786e\u8ba4")
$questionCuePattern = "\u8bf7\u9009\u62e9|\u8bf7\u786e\u8ba4|\u9009\u62e9\u4e00\u4e2a|\u9009\u9879|\u4f60\u5e0c\u671b|\u4f60\u60f3|\u662f\u5426|\u8981\u4e0d\u8981|\u54ea\u4e2a|\u54ea\u79cd|\u56de\u590d|choose|select|confirm|which"
$optionLinePattern = "(?m)^\s*(?:[-*]\s+|\d+[.)\u3001]\s*|[A-Da-d][.)\u3001]\s*)"
$fullWidthQuestionMark = [regex]::Unescape("\uff1f")

function Get-EventValue {
    param(
        [object]$EventData,
        [string]$Name
    )

    if ($null -ne $EventData -and $EventData.PSObject.Properties.Name -contains $Name) {
        return $EventData.$Name
    }
    return $null
}

function Get-SafeSessionId {
    param([object]$EventData)

    $raw = Get-EventValue $EventData "session_id"
    if (-not $raw) { $raw = Get-EventValue $EventData "conversation_id" }
    if (-not $raw) { $raw = $env:CODEBUDDY_SESSION_ID }
    if (-not $raw) { $raw = Get-EventValue $EventData "transcript_path" }
    if (-not $raw) { $raw = (Get-Location).Path }

    $value = [string]$raw
    $readable = ($value -replace "[^a-zA-Z0-9._-]+", "-").Trim("-")
    if ($readable -and $readable.Length -le 96) {
        return $readable
    }

    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = [System.Text.Encoding]::UTF8.GetBytes($value)
        $hash = $sha256.ComputeHash($bytes)
        return ([System.BitConverter]::ToString($hash) -replace "-", "").Substring(0, 24).ToLower()
    } finally {
        $sha256.Dispose()
    }
}

function Test-Truthy {
    param([object]$Value)
    return $Value -eq $true -or ([string]$Value).ToLower() -in @("1", "true", "yes")
}

function Get-TextFragments {
    param([object]$Value)

    if ($null -eq $Value) { return @() }
    if ($Value -is [string]) { return @($Value) }
    if ($Value -is [System.Collections.IEnumerable] -and $Value -isnot [PSCustomObject]) {
        return @($Value | ForEach-Object { Get-TextFragments $_ })
    }

    $fragments = @()
    foreach ($name in @("content", "message", "text")) {
        if ($Value.PSObject.Properties.Name -contains $name) {
            $fragments += Get-TextFragments $Value.$name
        }
    }
    return $fragments
}

function Get-AssistantText {
    param([object]$Record)

    if ($null -eq $Record) { return "" }
    $message = Get-EventValue $Record "message"
    $messageRole = Get-EventValue $message "role"
    $recordType = Get-EventValue $Record "type"
    $recordRole = Get-EventValue $Record "role"
    if ($recordType -ne "assistant" -and $recordRole -ne "assistant" -and $messageRole -ne "assistant") {
        return ""
    }
    return (Get-TextFragments $(if ($message) { $message } else { $Record })) -join "`n"
}

function Get-LatestAssistantText {
    param([object]$EventData)

    $lastAssistantMessage = [string](Get-EventValue $EventData "last_assistant_message")
    if ($lastAssistantMessage.Trim()) {
        return $lastAssistantMessage
    }

    $transcriptPath = [string](Get-EventValue $EventData "transcript_path")
    if (-not $transcriptPath -or -not (Test-Path $transcriptPath -PathType Leaf)) {
        return ""
    }
    try {
        $content = [System.IO.File]::ReadAllText($transcriptPath)
        try {
            $records = @($content | ConvertFrom-Json)
        } catch {
            $records = @(
                $content -split "`r?`n" |
                    ForEach-Object {
                        try { $_ | ConvertFrom-Json } catch { $null }
                    } |
                    Where-Object { $null -ne $_ }
            )
        }
        for ($index = $records.Count - 1; $index -ge 0; $index--) {
            $text = Get-AssistantText $records[$index]
            if ($text) { return $text }
        }
    } catch {
        return ""
    }
    return ""
}

function Test-StopWaitsForUser {
    param([object]$EventData)

    $text = (Get-LatestAssistantText $EventData).Trim()
    if (-not $text -or $text -notmatch $questionCuePattern) {
        return $false
    }
    $optionMatches = [regex]::Matches($text, $optionLinePattern)
    return $optionMatches.Count -ge 2 -or $text.EndsWith("?") -or $text.EndsWith($fullWidthQuestionMark)
}

function Set-EventAwareState {
    param([object]$EventData)

    $eventName = [string](Get-EventValue $EventData "hook_event_name")
    if ($eventName -eq "PermissionRequest") {
        $script:State = "waiting"
        $script:Message = $textWaitingPermission
    } elseif ($eventName -eq "Elicitation") {
        $script:State = "waiting"
        $script:Message = $textWaitingInput
    } elseif ($eventName -eq "PermissionDenied") {
        $script:State = "error"
        $script:Message = $textPermissionDenied
    } elseif ($eventName -eq "PostToolUseFailure" -or $eventName -eq "StopFailure") {
        $script:State = "error"
        $script:Message = $textToolFailed
    } elseif ($eventName -eq "PreToolUse") {
        $toolName = ([string](Get-EventValue $EventData "tool_name") -replace "[^a-zA-Z0-9]+", "").ToLower()
        $toolInput = Get-EventValue $EventData "tool_input"
        if ($toolName -in @("askuserquestion", "askuser", "requestuserinput", "elicitation")) {
            $script:State = "waiting"
            $script:Message = $textWaitingChoice
        } elseif (Test-Truthy (Get-EventValue $toolInput "requires_approval")) {
            $script:State = "waiting"
            $script:Message = $textWaitingPermission
        }
    } elseif ($eventName -eq "PostToolUse") {
        $toolResponse = Get-EventValue $EventData "tool_response"
        $exitCode = Get-EventValue $toolResponse "exitCode"
        if ($null -eq $exitCode) { $exitCode = Get-EventValue $toolResponse "exit_code" }
        if ((Test-Truthy (Get-EventValue $toolResponse "is_error")) -or ($null -ne $exitCode -and [int]$exitCode -ne 0)) {
            $script:State = "error"
            $script:Message = $textToolFailed
        }
    } elseif ($eventName -eq "Stop" -and (Test-StopWaitsForUser $EventData)) {
        $script:State = "waiting"
        $script:Message = $textWaitingChoiceOrConfirmation
    }
}

try {
    $inputJson = $payloadText
    $eventData = if ($inputJson) { $inputJson | ConvertFrom-Json } else { [PSCustomObject]@{} }
} catch {
    $eventData = [PSCustomObject]@{}
}

if ($NotificationOnly) {
    $notificationType = [string](Get-EventValue $eventData "notification_type")
    switch ($notificationType) {
        "permission_prompt" {
            $State = "waiting"
            $Message = $textWaitingPermission
        }
        "elicitation_dialog" {
            $State = "waiting"
            $Message = $textWaitingInput
        }
        "idle_prompt" {
            $State = "completed"
            $Message = $textCompleted
        }
        default {
            exit 0
        }
    }
}

if ($Client -in @('codex','workbuddy')) {
    if (-not (Get-EventValue $eventData 'session_id') -and -not (Get-EventValue $eventData 'conversation_id')) {
        if ($EmitEmptyJson) { [Console]::Out.WriteLine('{}') }
        exit 0
    }
} else {
    Set-EventAwareState $eventData
}
New-Item -ItemType Directory -Path $sessionsDir -Force | Out-Null
$sessionFile = Join-Path $sessionsDir "$Client-$(Get-SafeSessionId $eventData).json"

if ($State -eq "idle") {
    Remove-Item $sessionFile -Force -ErrorAction SilentlyContinue
    if ($EmitEmptyJson) {
        [Console]::Out.WriteLine("{}")
    }
    exit 0
}

$epoch = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()
$createdAt = $epoch
if (Test-Path $sessionFile -PathType Leaf) {
    try {
        $existing = Get-Content $sessionFile -Raw | ConvertFrom-Json
        $existingCreatedAt = Get-EventValue $existing "created_at"
        if (-not $existingCreatedAt) { $existingCreatedAt = Get-EventValue $existing "timestamp" }
        if ($existingCreatedAt) { $createdAt = [long]$existingCreatedAt }
    } catch {
        $createdAt = $epoch
    }
}
$cwd = Get-EventValue $eventData "cwd"
if (-not $cwd) { $cwd = (Get-Location).Path }
$content = @{
    client = $Client
    state = $State
    message = $Message
    cwd = [string]$cwd
    timestamp = $epoch
    created_at = $createdAt
} | ConvertTo-Json -Compress

$temporaryFile = "$sessionFile.tmp.$PID"
[System.IO.File]::WriteAllText($temporaryFile, $content, $utf8WithoutBom)
Move-Item -Path $temporaryFile -Destination $sessionFile -Force
if ($EmitEmptyJson) {
    [Console]::Out.WriteLine("{}")
}
