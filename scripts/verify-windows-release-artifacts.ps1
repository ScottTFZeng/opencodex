param(
  [Parameter(Mandatory = $true)][string]$Installer,
  [Parameter(Mandatory = $true)][string]$Signature,
  [Parameter(Mandatory = $true)][string]$UpdaterPublicKey,
  [string[]]$Application = @(),
  [switch]$VerifyInstallerAuthenticode,
  [string]$ExpectedSignerIdentity = "",
  [string]$LatestJson = "",
  [string]$ExpectedVersion = "",
  [string]$ExpectedInstallerUrl = ""
)

$ErrorActionPreference = "Stop"
$temporary = Join-Path $env:RUNNER_TEMP ("opencodex-minisign-" + [guid]::NewGuid().ToString("N"))
$decodedKey = "$temporary.pub"
$decodedSignature = "$temporary.sig"

function Assert-AuthenticodeIdentity([string]$Path, [string]$Expected) {
  $signature = Get-AuthenticodeSignature "$Path"
  if ($signature.Status -ne "Valid") { throw "Authenticode status for $Path is $($signature.Status)." }
  if ([string]::IsNullOrWhiteSpace($Expected)) { throw "Expected Authenticode signer identity is required." }
  if ($Expected.StartsWith("thumbprint:", [System.StringComparison]::OrdinalIgnoreCase)) {
    $thumbprint = $Expected.Substring("thumbprint:".Length).Replace(" ", "")
    if ($signature.SignerCertificate.Thumbprint -ne $thumbprint) { throw "Authenticode signer thumbprint did not match for $Path." }
  } elseif ($signature.SignerCertificate.Subject -ne $Expected) {
    throw "Authenticode signer subject did not match for $Path."
  }
  signtool verify /pa /v "$Path"
  if ($LASTEXITCODE -ne 0) { throw "signtool verification failed for $Path." }
}

try {
  [IO.File]::WriteAllBytes($decodedKey, [Convert]::FromBase64String($UpdaterPublicKey.Trim()))
  [IO.File]::WriteAllBytes($decodedSignature, [Convert]::FromBase64String((Get-Content "$Signature" -Raw).Trim()))
  minisign -Vm "$Installer" -x "$decodedSignature" -p "$decodedKey"
  if ($LASTEXITCODE -ne 0) { throw "Updater signature verification failed." }

  if ($VerifyInstallerAuthenticode) {
    Assert-AuthenticodeIdentity $Installer $ExpectedSignerIdentity
  }
  foreach ($application in $Application) { Assert-AuthenticodeIdentity $application $ExpectedSignerIdentity }

  if ($LatestJson) {
    $env:DESKTOP_VALIDATE_LATEST_JSON = "1"
    $env:DESKTOP_LATEST_JSON_PATH = $LatestJson
    $env:DESKTOP_INSTALLER_SIG_PATH = $Signature
    $env:DESKTOP_RELEASE_VERSION = $ExpectedVersion
    $env:DESKTOP_INSTALLER_NAME = [IO.Path]::GetFileName($Installer)
    $env:DESKTOP_INSTALLER_URL = $ExpectedInstallerUrl
    bun scripts/desktop-latest-json.ts
    if ($LASTEXITCODE -ne 0) { throw "latest.json validation failed." }
  }
} finally {
  Remove-Item -Force -ErrorAction SilentlyContinue "$decodedKey", "$decodedSignature"
}
