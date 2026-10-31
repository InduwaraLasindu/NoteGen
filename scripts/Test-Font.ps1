#Requires -Version 7.0
[CmdletBinding()]
param([string]$Font='build/font-evidence/NoteGen-test.ttf',[string]$Preview='build/font-evidence/windows-font.png')
$ErrorActionPreference='Stop'
$Font=(Resolve-Path $Font).Path
Add-Type -AssemblyName System.Drawing
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NoteGenFontGate {
 [DllImport("gdi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern int AddFontResourceExW(string file, uint flags, IntPtr reserved);
 [DllImport("gdi32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern bool RemoveFontResourceExW(string file, uint flags, IntPtr reserved);
}
'@
$loaded=[NoteGenFontGate]::AddFontResourceExW($Font,16,[IntPtr]::Zero)
if($loaded -lt 1){throw 'Windows GDI rejected the exported TrueType font'}
$fonts=[Drawing.Text.PrivateFontCollection]::new()
try {
 $fonts.AddFontFile($Font)
 if($fonts.Families.Count -lt 1){throw 'Private font collection could not load the font'}
 $typeface=[Drawing.Font]::new($fonts.Families[0],72,[Drawing.FontStyle]::Regular,[Drawing.GraphicsUnit]::Pixel)
 $bitmap=[Drawing.Bitmap]::new(180,140)
 $graphics=[Drawing.Graphics]::FromImage($bitmap)
 try {
  $graphics.Clear([Drawing.Color]::White)
  $graphics.DrawString('r',$typeface,[Drawing.Brushes]::Black,10,10)
  $ink=0
  for($y=0;$y -lt $bitmap.Height;$y++){for($x=0;$x -lt $bitmap.Width;$x++){if($bitmap.GetPixel($x,$y).R -lt 220){$ink++}}}
  if($ink -lt 50){throw 'Windows loaded the font but did not render visible glyph ink'}
  New-Item -ItemType Directory -Force (Split-Path $Preview -Parent) | Out-Null
  $bitmap.Save([IO.Path]::GetFullPath($Preview),[Drawing.Imaging.ImageFormat]::Png)
  Write-Host "Native Windows font load and glyph rendering passed: $ink pixels"
 } finally {$graphics.Dispose();$bitmap.Dispose();$typeface.Dispose()}
} finally {$fonts.Dispose();[NoteGenFontGate]::RemoveFontResourceExW($Font,16,[IntPtr]::Zero) | Out-Null}
