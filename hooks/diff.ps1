param([string] $A, [string] $B)
Add-Type -AssemblyName System.Drawing
$ia = [System.Drawing.Bitmap]::FromFile($A)
$ib = [System.Drawing.Bitmap]::FromFile($B)
if ($ia.Width -ne $ib.Width -or $ia.Height -ne $ib.Height) {
    Write-Output ("SIZE MISMATCH " + $ia.Width + "x" + $ia.Height + " vs " + $ib.Width + "x" + $ib.Height)
} else {
    $rect = New-Object System.Drawing.Rectangle(0, 0, $ia.Width, $ia.Height)
    $da = $ia.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $db = $ib.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $n = $da.Width * $da.Height
    $size = $n * 4
    $ba = [byte[]]::new($size)
    $bb = [byte[]]::new($size)
    [System.Runtime.InteropServices.Marshal]::Copy([IntPtr]$da.Scan0, $ba, 0, $size)
    [System.Runtime.InteropServices.Marshal]::Copy([IntPtr]$db.Scan0, $bb, 0, $size)
    $ia.UnlockBits($da); $ib.UnlockBits($db)
    $sum = 0L
    for ($i = 0; $i -lt $ba.Length; $i += 4) {
        $d = [Math]::Abs($ba[$i] - $bb[$i]) + [Math]::Abs($ba[$i+1] - $bb[$i+1]) + [Math]::Abs($ba[$i+2] - $bb[$i+2])
        $sum += $d
    }
    Write-Output ("MEAN " + [Math]::Round($sum / $n, 3))
    Write-Output ("RAW " + $sum)
}
$ia.Dispose(); $ib.Dispose()
