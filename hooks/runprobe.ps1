param([int] $Seconds = 12, [string] $Args = "")
$env:RUST_BACKTRACE = "1"
$argList = @()
if ($Args -ne "") {
    $argList = @($Args -split ",") | Where-Object { $_ -ne "" }
}
if ($argList.Count -gt 0) {
    $proc = Start-Process "D:\counter\target\debug\counter.exe" -ArgumentList $argList -PassThru
} else {
    $proc = Start-Process "D:\counter\target\debug\counter.exe" -PassThru
}
Start-Sleep -Seconds $Seconds
if ($proc.HasExited) {
    Write-Output ("EXITED")
    Get-Content "D:\counter\run_out.txt" -Tail 6
    Get-Content "D:\counter\run_err.txt" -Tail 6
} else {
    Write-Output ("ALIVE pid=" + $proc.Id)
    Stop-Process -Id $proc.Id -Force
}
