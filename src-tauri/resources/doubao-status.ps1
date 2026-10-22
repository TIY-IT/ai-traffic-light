param([int]$ParentPid)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
$expression = '__DOUBAO_EXPRESSION__'
while (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue) {
  $aggregate = [pscustomobject]@{ connected = $false; active = 0; ended = 0 }
  try {
    $targets = Invoke-RestMethod 'http://127.0.0.1:19224/json/list' -TimeoutSec 5
    $pages = @($targets | Where-Object { $_.type -eq 'page' -and $_.url -like 'doubao://doubao-chat*' })
    foreach ($target in $pages) {
      $socket = $null
      $cancel = New-Object System.Threading.CancellationTokenSource
      $cancel.CancelAfter(5000)
      try {
        if ($target.webSocketDebuggerUrl -notmatch '^ws://127\.0\.0\.1:19224/devtools/page/[A-Za-z0-9-]+$') { throw 'Invalid target url' }
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
        $value = $response.result.result.value
        if ($value.connected) { $aggregate.connected = $true; $aggregate.active += [int]$value.active; $aggregate.ended += [int]$value.ended }
      } catch { }
      finally { if ($socket) { $socket.Dispose() }; $cancel.Dispose() }
    }
    [Console]::WriteLine((ConvertTo-Json -InputObject @($aggregate) -Depth 5 -Compress))
  } catch { [Console]::WriteLine('[]') }
  Start-Sleep -Milliseconds 500
}
