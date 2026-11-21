# WonderLand model weights downloader.
# Usage:
#   powershell -ExecutionPolicy Bypass -File models\download.ps1            # download all
#   powershell -ExecutionPolicy Bypass -File models\download.ps1 -Only u2netp   # download one
#
# Heavy models use the SAME ModelScope mirror as the in-app downloader
# (Settings -> Model plugins -> Model download), verified 2026-11-21;
# the app checks SHA-256 on those files. Light trio keeps original upstreams.
param(
    [string]$Dir = "$PSScriptRoot",
    [string]$Only = ""
)

$ProgressPreference = 'SilentlyContinue'
if (-not (Test-Path $Dir)) { New-Item -ItemType Directory -Path $Dir | Out-Null }

# name -> url (saved as <name>.onnx in models/)
$Models = [ordered]@{
    # OpenCV Zoo (Apache-2.0) - face detection (bundled with installer)
    "face_detection_yunet_2023mar" = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
    # rembg release assets (U^2-Net official weights, Apache-2.0 project) - 4.4 MB (bundled)
    "u2netp" = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx"
    # Xenova/modnet (MODNet photographic portrait matting, Apache-2.0) - 25 MB (bundled)
    "modnet" = "https://huggingface.co/Xenova/modnet/resolve/main/onnx/model.onnx"
    # Heavy models - ModelScope direct link (ureq/Invoke-WebRequest follow the 302 to CDN).
    "BEN2_Base" = "https://modelscope.cn/api/v1/models/laosan2026/cocole-matting-onnx/repo?Revision=master&FilePath=BEN2_Base.onnx"
    "birefnet_1024" = "https://modelscope.cn/api/v1/models/laosan2026/cocole-matting-onnx/repo?Revision=master&FilePath=birefnet_1024.onnx"
    "model_fp16" = "https://modelscope.cn/api/v1/models/laosan2026/cocole-matting-onnx/repo?Revision=master&FilePath=model_fp16.onnx"
}

foreach ($name in $Models.Keys) {
    if ($Only -and $name -notlike "*$Only*") { continue }
    $out = Join-Path $Dir "$name.onnx"
    if (Test-Path $out) {
        Write-Host "[skip] $name.onnx already exists" -ForegroundColor DarkGray
        continue
    }
    Write-Host "[get ] $name.onnx <- $($Models[$name])" -ForegroundColor Cyan
    try {
        Invoke-WebRequest -Uri $Models[$name] -OutFile $out -UseBasicParsing
        $mb = [math]::Round((Get-Item $out).Length / 1MB, 1)
        Write-Host "[done] $name.onnx ($mb MB)" -ForegroundColor Green
    } catch {
        Write-Host "[FAIL] $name : $_" -ForegroundColor Red
        if (Test-Path $out) { Remove-Item $out -Force }
    }
}
Write-Host "`nAll set. Restart WonderLand and hit the refresh button in the model dropdown."
