Add-Type -AssemblyName System.Drawing
$sizes = @(192, 512, 180)
foreach ($s in $sizes) {
    $bmp = New-Object System.Drawing.Bitmap($s, $s)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.Clear([System.Drawing.Color]::FromArgb(15, 16, 20))
    $brush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(91, 80, 232))
    $p = [int]($s * 0.2)
    $d = [int]($s * 0.6)
    $g.FillEllipse($brush, $p, $p, $d, $d)
    $g.Dispose()
    $bmp.Save("D:\counter\cranpose-pwa\demo\icon-$s.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}
Get-ChildItem 'D:\counter\cranpose-pwa\demo' -Filter 'icon-*.png' | Select-Object Name, Length
