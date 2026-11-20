param([string] $Out = "D:\counter\shots\tap1.png")
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M2 {
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, System.IntPtr e);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetClientRect(System.IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(System.IntPtr h, ref P p);
  public struct R { public int Left, Top, Right, Bottom; }
  public struct P { public int X, Y; }
}
"@
$p = Get-Process counter | Where-Object { $_.MainWindowTitle -ne "" } | Select-Object -First 1
$h = $p.MainWindowHandle
[M2]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 250
$o = New-Object M2+R
[M2]::GetClientRect($h, [ref]$o) | Out-Null
$sp = New-Object M2+P
$sp.X = 0; $sp.Y = 0
[M2]::ClientToScreen($h, [ref]$sp) | Out-Null
$x = $sp.X + [int]($o.Right / 2.0)
$y = $sp.Y + [int]($o.Bottom / 2.0)
[M2]::SetCursorPos($x, $y) | Out-Null
Start-Sleep -Milliseconds 150
[M2]::mouse_event(0x0002, 0, 0, 0, [System.IntPtr]::Zero) | Out-Null
[M2]::mouse_event(0x0004, 0, 0, 0, [System.IntPtr]::Zero) | Out-Null
Write-Output "clicked center $x,$y"
