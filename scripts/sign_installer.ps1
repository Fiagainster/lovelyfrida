# LovelyFrida 安装包签名（文档10 C2 脚手架）
# 用法：python scripts\build_sidecar.py && npx tauri build 之后——
#   powershell -ExecutionPolicy Bypass -File scripts\sign_installer.ps1 <安装包或exe 路径>
#
# 证书准备（二选一）：
#   ① OV/EV 代码签名证书导入当前用户证书库（Cert:\CurrentUser\My，用途=代码签名）
#   ② 内部测试：New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=LovelyFrida"
#      （自签仅消除"无发布者"提示，不消除 SmartScreen 名誉警告——正式分发用 CA 证书）
param([Parameter(Mandatory = $true)][string]$Path)

if (-not (Test-Path $Path)) { Write-Error "文件不存在：$Path"; exit 1 }
if (-not (Get-Command signtool -ErrorAction SilentlyContinue)) {
    Write-Error "signtool 不在 PATH：安装 Windows SDK（或使用 /c0000 路径下的 Windows Kits\10\bin\x64\signtool.exe）"
    exit 1
}

$cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Sort-Object NotAfter -Descending | Select-Object -First 1
if (-not $cert) {
    Write-Error "当前用户证书库没有代码签名证书（见脚本头注释的证书准备）"
    exit 1
}
Write-Host "使用证书：$($cert.Subject)（指纹 $($cert.Thumbprint)，有效期至 $($cert.NotAfter)）"

# SHA256 摘要 + RFC3161 时间戳（证书过期后签名仍有效）
& signtool sign /fd SHA256 /td SHA256 /tr http://timestamp.digicert.com /sha1 $cert.Thumbprint $Path
if ($LASTEXITCODE -ne 0) { Write-Error "签名失败"; exit $LASTEXITCODE }

& signtool verify /pa /all $Path
if ($LASTEXITCODE -ne 0) { Write-Error "验签失败"; exit $LASTEXITCODE }
Write-Host "✅ 已签名并验签：$Path"
