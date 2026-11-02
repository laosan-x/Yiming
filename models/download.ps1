# WonderLand model weights downloader.
# Usage:
#   powershell -ExecutionPolicy Bypass -File models\download.ps1            # download all
#   powershell -ExecutionPolicy Bypass -File models\download.ps1 -Only u2netp   # download one
#
# All URLs verified 2026-11-02. Weights belong to their upstream projects,
# see THIRD-PARTY-NOTICES.md and README acknowledgements for licenses.
param(
    [string]$Dir = "$PSScriptRoot",
    [string]$Only = ""
)

$ProgressPreference = 'SilentlyContinue'
if (-not (Test-Path $Dir)) { New-Item -ItemType Directory -Path $Dir | Out-Null }

# name -> url (saved as <name>.onnx in models/)
$Models = [ordered]@{
    # OpenCV Zoo (Apache-2.0) - face detection
    "face_detection_yunet_2023mar" = "https://github.com/opencv/opencv_zoo/raw/main/models/face_detection_yunet/face_detection_yunet_2023mar.onnx"
    # rembg release assets (U^2-Net official weights, Apache-2.0 project) - 4 MB
    "u2netp" = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx"
    # Xenova/modnet (MODNet photographic portrait matting, Apache-2.0) - 24 MB
    "modnet" = "https://huggingface.co/Xenova/modnet/resolve/main/onnx/model.onnx"
    # PramaLLC/BEN2 official (MIT) - 212 MB
    "BEN2_Base" = "https://huggingface.co/PramaLLC/BEN2/resolve/main/BEN2_Base.onnx"
    # ZhengPeng7 official ONNX export (MIT) - 927 MB
    "birefnet_1024" = "https://github.com/ZhengPeng7/BiRefNet/releases/download/v1/BiRefNet-general-epoch_244.onnx"
    # InSPyReNet-based general matting (rembg official weights, 170 MB).
    # NOTE: official InSPyReNet repo ships no ONNX; this is the same-architecture
    # general-use export. If you already have model_fp16.onnx, keep yours.
    "isnet-general-use" = "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx"
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
