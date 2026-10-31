# Watchdog for zn-server on VPS
$serverExe = "C:\zona\zn-server.exe"
$workDir = "C:\zona"
$logFile = "C:\zona\server.log"

while ($true) {
    $proc = Get-Process -Name "zn-server" -ErrorAction SilentlyContinue
    if (-not $proc) {
        $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
        Add-Content -Path $logFile -Value "$ts - server not running, starting..."
        Start-Process -FilePath $serverExe -WorkingDirectory $workDir -WindowStyle Hidden
        Start-Sleep -Seconds 5
    }
    Start-Sleep -Seconds 10
}
