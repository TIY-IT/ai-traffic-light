param([int]$ParentPid)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$expression = '__COZE_EXPRESSION__'
while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) {
  $socket = $null
  $cancel = New-Object System.Threading.CancellationTokenSource
  $cancel.CancelAfter(3000)
  try {
    $targets = Invoke-RestMethod 'http://127.0.0.1:19222/json/list' -TimeoutSec 2
    $target = @($targets | Where-Object { $_.type -eq 'page' -and $_.url -like 'coze-desktop://-/*' }) | Select-Object -First 1
    if (!$target -or $target.webSocketDebuggerUrl -notmatch '^ws://127\.0\.0\.1:19222/devtools/page/[A-Za-z0-9-]+$') { throw 'No Coze target' }
    $socket = New-Object System.Net.WebSockets.ClientWebSocket
    $null = $socket.ConnectAsync([Uri]$target.webSocketDebuggerUrl,$cancel.Token).GetAwaiter().GetResult()
    $request = @{id=1;method='Runtime.evaluate';params=@{expression=$expression;returnByValue=$true}} | ConvertTo-Json -Depth 5 -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($request)
    $null = $socket.SendAsync([ArraySegment[byte]]::new($bytes),[Net.WebSockets.WebSocketMessageType]::Text,$true,$cancel.Token).GetAwaiter().GetResult()
    $buffer = New-Object byte[] 65536
    $stream = New-Object IO.MemoryStream
    do {
      $result = $socket.ReceiveAsync([ArraySegment[byte]]::new($buffer),$cancel.Token).GetAwaiter().GetResult()
      $stream.Write($buffer,0,$result.Count)
      if ($stream.Length -gt 262144) { throw 'Response too large' }
    } while (!$result.EndOfMessage)
    $response = [Text.Encoding]::UTF8.GetString($stream.ToArray()) | ConvertFrom-Json
    $stream.Dispose()
    if ($response.id -ne 1 -or $response.error -or $response.result.exceptionDetails) { throw 'Invalid evaluation' }
    $rows = @($response.result.result.value)
    [Console]::WriteLine((ConvertTo-Json -InputObject $rows -Depth 5 -Compress))
  } catch { [Console]::WriteLine('[]') }
  finally { if ($socket) { $socket.Dispose() }; $cancel.Dispose() }
  Start-Sleep -Milliseconds 500
}
