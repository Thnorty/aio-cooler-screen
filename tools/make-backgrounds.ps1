# Renders the original 480x480 backgrounds for the round cooler screen, as looping animations.
# Each background is drawn as $Frames frames; every motion runs a whole number of cycles per
# loop, so the last frame leads straight back into the first. Frame 0 is saved as the still
# (name.png) and the frames are encoded to name.mp4 in the format the cooler plays
# (H.264, 20 fps, as media.rs produces). Needs ffmpeg on PATH.
param([string]$OutDir, [int]$Frames = 200)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
New-Item -ItemType Directory -Force $OutDir | Out-Null
$OutDir = (Resolve-Path $OutDir).Path
$ffmpeg = (Get-Command ffmpeg -ErrorAction SilentlyContinue).Source
if (-not $ffmpeg) { throw 'ffmpeg is not on PATH; it is needed to encode the animations.' }
$S = 480
$TAU = 2 * [math]::PI

function New-Canvas { $b = [Drawing.Bitmap]::new($S, $S); $g = [Drawing.Graphics]::FromImage($b); $g.SmoothingMode = 'AntiAlias'; $g.InterpolationMode = 'HighQualityBicubic'; $g.CompositingQuality = 'HighQuality'; return @($b, $g) }
function C([int]$a, [int]$r, [int]$gg, [int]$bb) { [Drawing.Color]::FromArgb([math]::Max(0, [math]::Min(255, $a)), $r, $gg, $bb) }

# Soft glow: a radial gradient blob that fades to transparent.
function Glow($g, [float]$cx, [float]$cy, [float]$r, $color, [int]$alpha) {
    $path = New-Object Drawing.Drawing2D.GraphicsPath
    $path.AddEllipse($cx - $r, $cy - $r, 2 * $r, 2 * $r)
    $br = New-Object Drawing.Drawing2D.PathGradientBrush $path
    $br.CenterColor = [Drawing.Color]::FromArgb([math]::Max(0, [math]::Min(255, $alpha)), $color)
    $br.SurroundColors = @([Drawing.Color]::FromArgb(0, $color))
    $g.FillPath($br, $path); $br.Dispose(); $path.Dispose()
}

function Fill-Radial($g, $inner, $outer) {
    $path = New-Object Drawing.Drawing2D.GraphicsPath
    $path.AddEllipse(-120, -120, $S + 240, $S + 240)
    $br = New-Object Drawing.Drawing2D.PathGradientBrush $path
    $br.CenterPoint = [Drawing.PointF]::new(240, 240)
    $br.CenterColor = $inner; $br.SurroundColors = @($outer)
    $g.FillRectangle($br, 0, 0, $S, $S); $br.Dispose(); $path.Dispose()
}

function Dot($g, [float]$x, [float]$y, [float]$r, $color) {
    $br = New-Object Drawing.SolidBrush $color; $g.FillEllipse($br, $x - $r, $y - $r, 2 * $r, 2 * $r); $br.Dispose()
}

# Twinkle factor for a star: whole cycles per loop so it repeats exactly.
function Twinkle([double]$phi, $star) { 0.55 + 0.45 * [math]::Sin($star.Speed * $phi + $star.Phase) }

function New-Stars($rng, [int]$count, [double]$maxY, [double]$rMin, [double]$rSpread, [double]$rPow, [int]$aMin, [int]$aSpread) {
    for ($i = 0; $i -lt $count; $i++) {
        [pscustomobject]@{
            X = $rng.NextDouble() * $S; Y = $rng.NextDouble() * $maxY
            R = [math]::Pow($rng.NextDouble(), $rPow) * $rSpread + $rMin
            A = $aMin + $rng.NextDouble() * $aSpread
            Speed = 1 + $rng.Next(3); Phase = $rng.NextDouble() * $TAU
        }
    }
}

# Renders one background: $static draws the parts that never move (once), $frame draws the
# rest for a loop phase phi in [0, 2pi). Writes name.png (frame 0) and name.mp4.
function Render([string]$name, [scriptblock]$static, [scriptblock]$frame) {
    $base, $g = New-Canvas; & $static $g; $g.Dispose()
    $tmp = Join-Path ([IO.Path]::GetTempPath()) "aio-bg-$name"
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
    New-Item -ItemType Directory $tmp | Out-Null
    for ($f = 0; $f -lt $Frames; $f++) {
        $b, $g = New-Canvas
        $g.DrawImageUnscaled($base, 0, 0)
        & $frame $g ($TAU * $f / $Frames)
        $g.Dispose()
        $b.Save((Join-Path $tmp ('f{0:D3}.png' -f $f)))
        if ($f -eq 0) { $b.Save("$OutDir\$name.png") }
        $b.Dispose()
    }
    $base.Dispose()
    & $ffmpeg -hide_banner -loglevel error -y -framerate 20 -i (Join-Path $tmp 'f%03d.png') `
        -c:v libx264 -profile:v high -pix_fmt yuv420p -b:v 1000k -maxrate 1500k -bufsize 2000k `
        -an -movflags +faststart "$OutDir\$name.mp4"
    if ($LASTEXITCODE) { throw "ffmpeg failed for $name" }
    Remove-Item -Recurse -Force $tmp
    "$name done"
}

# 1) Nebula: deep space with drifting, breathing gas clouds and twinkling stars.
$rng = New-Object Random 7
$nebulaStars = New-Stars $rng 260 $S 0.4 1.8 3 90 165
$blobs = @(@(150, 170, 210, (C 255 120 60 220), 120), @(330, 300, 230, (C 255 30 140 230), 110), @(300, 140, 150, (C 255 230 70 160), 80), @(170, 350, 160, (C 255 40 200 190), 70))
Render 'nebula' { param($g) Fill-Radial $g (C 255 22 18 48) (C 255 4 4 12) } {
    param($g, $phi)
    # The clouds swirl on wide circles (alternating direction) and breathe in size and brightness.
    for ($j = 0; $j -lt $blobs.Count; $j++) {
        $bl = $blobs[$j]; $dir = if ($j % 2) { -1 } else { 1 }; $p = $dir * $phi + $j * 1.6
        Glow $g ($bl[0] + 55 * [math]::Cos($p)) ($bl[1] + 45 * [math]::Sin($p)) ($bl[2] * (1 + 0.15 * [math]::Sin($phi + $j * 2.1))) $bl[3] ($bl[4] * (0.7 + 0.3 * [math]::Sin($phi + $j)))
    }
    foreach ($st in $nebulaStars) { Dot $g $st.X $st.Y $st.R (C ($st.A * (0.15 + 0.85 * (0.5 + 0.5 * [math]::Sin($st.Speed * $phi + $st.Phase)))) 255 255 255) }
    $n = 0
    foreach ($star in @(@(110, 120), @(360, 90), @(400, 380), @(90, 330))) {
        Glow $g $star[0] $star[1] (10 + 4 * [math]::Sin(2 * $phi + $n * 1.7)) (C 255 255 255 255) (220 * (0.45 + 0.55 * [math]::Sin(2 * $phi + $n * 1.7))); $n++
    }
    # A shooting star once per loop, crossing during the first fifth of it.
    $u = $phi / $TAU / 0.2
    if ($u -gt 0 -and $u -lt 1) {
        $hx = 60 + 360 * $u; $hy = 60 + 160 * $u
        for ($k = 0; $k -lt 12; $k++) {
            $pen = [Drawing.Pen]::new((C ([int]((230 - $k * 19) * [math]::Sin([math]::PI * $u))) 255 255 255), [float](2.4 - $k * 0.15))
            $g.DrawLine($pen, [float]($hx - $k * 9), [float]($hy - $k * 4), [float]($hx - ($k + 1) * 9), [float]($hy - ($k + 1) * 4)); $pen.Dispose()
        }
    }
}

# 2) Aurora: light bands that ripple, rise and fall over a dark sky and still mountains.
$rng = New-Object Random 11
$auroraStars = New-Stars $rng 120 200 0.3 1.1 1 80 140
$mountain = @(([Drawing.PointF]::new(0, 480)), ([Drawing.PointF]::new(0, 400)))
for ($x = 0; $x -le 480; $x += 24) { $mountain += [Drawing.PointF]::new($x, 400 - 40 * [math]::Abs([math]::Sin($x / 70.0)) - 20 * $rng.NextDouble()) }
$mountain += [Drawing.PointF]::new(480, 480)
$bands = @(@(0, 230, (C 255 60 240 170)), @(40, 270, (C 255 70 180 255)), @(-30, 300, (C 255 170 110 255)))
Render 'aurora' { param($g) Fill-Radial $g (C 255 8 34 44) (C 255 2 8 14) } {
    param($g, $phi)
    for ($bi = 0; $bi -lt 3; $bi++) {
        $band = $bands[$bi]
        $lift = 8 * [math]::Sin($phi + $bi * 2.1)
        $shine = 0.8 + 0.25 * [math]::Sin(2 * $phi + $bi * 2.1)
        for ($k = 0; $k -lt 26; $k++) {
            $off = $band[0] + $k * 3.2
            # Two waves travelling in opposite directions, one cycle each per loop.
            $pts = for ($x = -20; $x -le 500; $x += 20) {
                [Drawing.PointF]::new($x, $band[1] + $lift + 55 * [math]::Sin(($x + $off) / 85.0 + $phi) - 30 * [math]::Cos(($x - $off) / 140.0 - $phi) - $k * 4)
            }
            $alpha = [math]::Min(255, [int]((26 * (1 - $k / 26.0) + 4) * $shine))
            $pen = [Drawing.Pen]::new([Drawing.Color]::FromArgb($alpha, $band[2]), [float](14 - $k * 0.4))
            $g.DrawCurve($pen, [Drawing.PointF[]]$pts, 0.5); $pen.Dispose()
        }
    }
    foreach ($st in $auroraStars) { Dot $g $st.X $st.Y $st.R (C ($st.A * (Twinkle $phi $st)) 230 250 255) }
    $br = New-Object Drawing.SolidBrush (C 255 3 10 14); $g.FillPolygon($br, [Drawing.PointF[]]$mountain); $br.Dispose()
}

# 3) Neon rings: glowing rings that breathe, with a bright spark circling each one.
$rings = @(@(228, (C 255 255 40 120), 3, 1), @(212, (C 255 0 220 255), 2, -1), @(150, (C 255 140 80 255), 1.5, 1))
Render 'neon-rings' {
    param($g)
    Fill-Radial $g (C 255 26 26 32) (C 255 6 6 9)
    for ($y = 0; $y -lt $S; $y += 6) {
        for ($x = (($y / 6) % 2) * 6; $x -lt $S; $x += 12) {
            $br = New-Object Drawing.SolidBrush (C 14 255 255 255); $g.FillRectangle($br, $x, $y, 6, 3); $br.Dispose()
        }
    }
} {
    param($g, $phi)
    for ($ri = 0; $ri -lt $rings.Count; $ri++) {
        $ring = $rings[$ri]
        $rad = $ring[0] + 3 * [math]::Sin($phi + $ri * 2.1)
        $glow = 0.75 + 0.25 * [math]::Sin(2 * $phi + $ri * 2.1)
        for ($w = 12; $w -ge 1; $w -= 1) {
            $pen = [Drawing.Pen]::new([Drawing.Color]::FromArgb([int]((10 + (12 - $w) * 4) * $glow), $ring[1]), [float]($ring[2] + $w * 2))
            $g.DrawEllipse($pen, 240 - $rad, 240 - $rad, 2 * $rad, 2 * $rad); $pen.Dispose()
        }
        $pen = New-Object Drawing.Pen ([Drawing.Color]::FromArgb(255, $ring[1])), $ring[2]
        $g.DrawEllipse($pen, 240 - $rad, 240 - $rad, 2 * $rad, 2 * $rad); $pen.Dispose()
        # One revolution per loop, alternating direction; a fading tail behind the spark.
        $angle = $ring[3] * $phi * 180 / [math]::PI + $ri * 120
        for ($t = 0; $t -lt 8; $t++) {
            $pen = [Drawing.Pen]::new([Drawing.Color]::FromArgb([int](150 - $t * 18), 255, 255, 255), [float]($ring[2] + 2))
            $pen.StartCap = 'Round'; $pen.EndCap = 'Round'
            $g.DrawArc($pen, 240 - $rad, 240 - $rad, 2 * $rad, 2 * $rad, [float]($angle - $ring[3] * $t * 5), [float]($ring[3] * 5)); $pen.Dispose()
        }
    }
    Glow $g 240 240 140 (C 255 120 60 255) (50 * (0.7 + 0.3 * [math]::Sin($phi)))
}

# 4) Sunset: a pulsing sun that bobs gently, swaying hills and a few twinkling stars.
$rng = New-Object Random 23
$sunsetStars = New-Stars $rng 40 150 0.3 1.0 1 60 120
$layers = @(@(330, (C 255 120 50 110), 30), @(370, (C 255 70 30 80), 24), @(410, (C 255 36 16 50), 18))
Render 'sunset' {
    param($g)
    $br = New-Object Drawing.Drawing2D.LinearGradientBrush ([Drawing.Point]::new(0, 0)), ([Drawing.Point]::new(0, 480)), (C 255 40 20 80), (C 255 255 120 80)
    $blend = New-Object Drawing.Drawing2D.ColorBlend 4
    $blend.Colors = @((C 255 28 16 70), (C 255 120 40 120), (C 255 240 100 90), (C 255 255 190 110))
    $blend.Positions = @(0.0, 0.45, 0.75, 1.0)
    $br.InterpolationColors = $blend
    $g.FillRectangle($br, 0, 0, $S, $S); $br.Dispose()
} {
    param($g, $phi)
    foreach ($st in $sunsetStars) { Dot $g $st.X $st.Y $st.R (C ($st.A * (Twinkle $phi $st)) 255 240 230) }
    $bob = 12 * [math]::Sin($phi)
    Glow $g 240 (300 + $bob) (170 + 25 * [math]::Sin(2 * $phi)) (C 255 255 210 140) (160 * (0.7 + 0.3 * [math]::Sin(2 * $phi)))
    $br = New-Object Drawing.SolidBrush (C 255 255 236 190); $g.FillEllipse($br, 180, 240 + $bob, 120, 120); $br.Dispose()
    for ($li = 0; $li -lt $layers.Count; $li++) {
        $l = $layers[$li]
        $sway = 0.9 * [math]::Sin($phi + $li * 2.1)
        $pts = @(([Drawing.PointF]::new(0, 480)))
        for ($x = 0; $x -le 480; $x += 20) { $pts += [Drawing.PointF]::new($x, $l[0] + $l[2] * [math]::Sin(($x + $l[0]) / 60.0 + $sway)) }
        $pts += [Drawing.PointF]::new(480, 480)
        $br = New-Object Drawing.SolidBrush $l[1]; $g.FillPolygon($br, [Drawing.PointF[]]$pts); $br.Dispose()
    }
}

# Preview sheet with round masks, as the cooler shows them.
$sheet = [Drawing.Bitmap]::new(1920, 480); $g = [Drawing.Graphics]::FromImage($sheet); $g.SmoothingMode = 'AntiAlias'
$g.Clear((C 255 24 24 27)); $i = 0
foreach ($n in 'nebula', 'aurora', 'neon-rings', 'sunset') {
    $img = [Drawing.Image]::FromFile("$OutDir\$n.png")
    $path = New-Object Drawing.Drawing2D.GraphicsPath; $path.AddEllipse($i * 480 + 10, 10, 460, 460); $g.SetClip($path)
    $g.DrawImage($img, $i * 480 + 10, 10, 460, 460); $g.ResetClip(); $img.Dispose(); $i++
}
$g.Dispose(); $sheet.Save("$OutDir\_preview.png"); $sheet.Dispose()
"done"
