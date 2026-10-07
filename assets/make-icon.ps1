# Draw the desk icon and pack a multi-size .ico plus a 256px PNG.
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

function New-RoundRect([System.Drawing.RectangleF]$rect, [single]$radius) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $radius * 2
    $path.AddArc($rect.X, $rect.Y, $d, $d, 180, 90) | Out-Null
    $path.AddArc(($rect.Right - $d), $rect.Y, $d, $d, 270, 90) | Out-Null
    $path.AddArc(($rect.Right - $d), ($rect.Bottom - $d), $d, $d, 0, 90) | Out-Null
    $path.AddArc($rect.X, ($rect.Bottom - $d), $d, $d, 90, 90) | Out-Null
    $path.CloseFigure()
    return $path
}

function Draw-Master {
    $size = 1024
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
    $g.Clear([System.Drawing.Color]::FromArgb(0, 0, 0, 0))

    $s = [single]$size
    $pad = $s * 0.05
    $body = New-Object System.Drawing.RectangleF $pad, $pad, ($s - 2 * $pad), ($s - 2 * $pad)
    $shape = New-RoundRect $body ($s * 0.23)
    $fill = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 32, 27, 22))
    $g.FillPath($fill, $shape)
    $edge = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 120, 92, 58)), ($s * 0.012)
    $g.DrawPath($edge, $shape)

    $cx = $s * 0.5
    $cy = $s * 0.52
    $radius = $s * 0.275
    $ring = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(255, 228, 177, 90)), ($s * 0.078)
    $ring.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
    $ring.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $g.DrawArc($ring, ($cx - $radius), ($cy - $radius), ($radius * 2), ($radius * 2), -50, 292)

    $tomato = $s * 0.112
    $red = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 226, 91, 74))
    $g.FillEllipse($red, ($cx - $tomato), ($cy - $tomato * 0.82), ($tomato * 2), ($tomato * 2))
    $leaf = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 78, 168, 112))
    $g.FillEllipse($leaf, ($cx - $tomato * 0.22), ($cy - $tomato * 1.62), ($tomato * 0.95), ($tomato * 0.58))
    $stem = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 58, 130, 88))
    $g.FillEllipse($stem, ($cx - $tomato * 0.12), ($cy - $tomato * 1.35), ($tomato * 0.28), ($tomato * 0.42))
    $shine = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(210, 255, 214, 196))
    $g.FillEllipse($shine, ($cx - $tomato * 0.62), ($cy - $tomato * 0.48), ($tomato * 0.48), ($tomato * 0.34))

    $g.Dispose()
    $fill.Dispose()
    $edge.Dispose()
    $ring.Dispose()
    $red.Dispose()
    $leaf.Dispose()
    $stem.Dispose()
    $shine.Dispose()
    $shape.Dispose()
    return $bmp
}

function New-SizedBitmap([System.Drawing.Image]$master, [int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::FromArgb(0, 0, 0, 0))
    $g.DrawImage($master, 0, 0, $size, $size)
    $g.Dispose()
    return $bmp
}

function Export-PngBytes([System.Drawing.Bitmap]$bmp) {
    $stream = New-Object System.IO.MemoryStream
    $bmp.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $stream.ToArray()
    $stream.Dispose()
    return ,$bytes
}

function Export-DibBytes([System.Drawing.Bitmap]$bmp) {
    $w = $bmp.Width
    $h = $bmp.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $w, $h
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $stride = [Math]::Abs($data.Stride)
    $raw = New-Object byte[] ($stride * $h)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $raw, 0, $raw.Length)
    $bmp.UnlockBits($data)

    $maskStride = [int]([Math]::Ceiling($w / 32.0) * 4)
    $xorLen = $w * $h * 4
    $maskLen = $maskStride * $h
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter $ms
    $bw.Write([uint32]40)
    $bw.Write([int32]$w)
    $bw.Write([int32]($h * 2))
    $bw.Write([uint16]1)
    $bw.Write([uint16]32)
    $bw.Write([uint32]0)
    $bw.Write([uint32]($xorLen + $maskLen))
    $bw.Write([int32]0)
    $bw.Write([int32]0)
    $bw.Write([uint32]0)
    $bw.Write([uint32]0)
    for ($y = $h - 1; $y -ge 0; $y--) {
        $row = $y * $stride
        for ($x = 0; $x -lt $w; $x++) {
            $i = $row + ($x * 4)
            $bw.Write($raw[$i])
            $bw.Write($raw[$i + 1])
            $bw.Write($raw[$i + 2])
            $bw.Write($raw[$i + 3])
        }
    }
    for ($y = $h - 1; $y -ge 0; $y--) {
        $row = $y * $stride
        $bits = New-Object byte[] $maskStride
        for ($x = 0; $x -lt $w; $x++) {
            $alpha = $raw[$row + ($x * 4) + 3]
            if ($alpha -lt 128) {
                $index = [int][Math]::Floor($x / 8)
                if ($index -lt 0 -or $index -ge $bits.Length) {
                    throw "mask w=$w x=$x index=$index len=$($bits.Length) stride=$maskStride"
                }
                $bit = [byte](128 / [Math]::Pow(2, ($x % 8)))
                $bits[$index] = [byte]($bits[$index] -bor $bit)
            }
        }
        $bw.Write($bits)
    }
    $bw.Flush()
    $bytes = $ms.ToArray()
    $bw.Dispose()
    return ,$bytes
}

$root = Split-Path -Parent $MyInvocation.MyCommand.Path
$master = Draw-Master
$sizes = @(16, 24, 32, 48, 64, 128, 256)
$images = foreach ($size in $sizes) {
    $bmp = New-SizedBitmap $master $size
    $png = Export-PngBytes $bmp
    $dib = Export-DibBytes $bmp
    $bmp.Dispose()
    [pscustomobject]@{ Size = $size; Png = $png; Bytes = $dib }
}
[IO.File]::WriteAllBytes((Join-Path $root "desk.png"), $images[-1].Png)

$out = New-Object System.IO.MemoryStream
$writer = New-Object System.IO.BinaryWriter $out
$writer.Write([uint16]0)
$writer.Write([uint16]1)
$writer.Write([uint16]$images.Count)
$offset = 6 + (16 * $images.Count)
foreach ($image in $images) {
    $dim = if ($image.Size -ge 256) { [byte]0 } else { [byte]$image.Size }
    $writer.Write($dim)
    $writer.Write($dim)
    $writer.Write([byte]0)
    $writer.Write([byte]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]32)
    $writer.Write([uint32]$image.Bytes.Length)
    $writer.Write([uint32]$offset)
    $offset += $image.Bytes.Length
}
foreach ($image in $images) {
    $payload = [byte[]]$image.Bytes
    $out.Write($payload, 0, $payload.Length)
}
$writer.Flush()
[IO.File]::WriteAllBytes((Join-Path $root "desk.ico"), $out.ToArray())
$writer.Dispose()
$master.Dispose()
"wrote desk.ico ($($images.Count) sizes) and desk.png"
