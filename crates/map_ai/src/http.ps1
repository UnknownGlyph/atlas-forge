$ErrorActionPreference = 'Stop'
$utf8 = New-Object System.Text.UTF8Encoding($false)
[Console]::InputEncoding = $utf8
[Console]::OutputEncoding = $utf8
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$request = ConvertFrom-Json -InputObject ([Console]::In.ReadLine())
$payload = $utf8.GetBytes([Console]::In.ReadToEnd())
Add-Type -AssemblyName System.Net.Http
$handler = New-Object Net.Http.HttpClientHandler
$handler.AllowAutoRedirect = $false
$handler.AutomaticDecompression = [Net.DecompressionMethods]::GZip -bor [Net.DecompressionMethods]::Deflate
$client = New-Object Net.Http.HttpClient($handler)
$message = New-Object Net.Http.HttpRequestMessage([Net.Http.HttpMethod]::Post, [uri]$request.url)
try {
    $client.Timeout = [TimeSpan]::FromSeconds($request.timeout)
    $message.Content = New-Object Net.Http.ByteArrayContent(,$payload)
    $message.Content.Headers.ContentType = [Net.Http.Headers.MediaTypeHeaderValue]::Parse('application/json; charset=utf-8')
    foreach ($header in $request.headers) {
        if (!$message.Headers.TryAddWithoutValidation([string]$header[0], [string]$header[1])) {
            $message.Content.Headers.Add([string]$header[0], [string]$header[1])
        }
    }
    $response = $client.SendAsync($message).GetAwaiter().GetResult()
    try {
        $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        [Console]::Out.WriteLine([int]$response.StatusCode)
        [Console]::Out.Write($utf8.GetString($bytes).TrimStart([char]0xfeff))
    } finally { $response.Dispose() }
} finally {
    $message.Dispose()
    $client.Dispose()
}
