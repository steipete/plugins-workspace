param(
  [Parameter(Mandatory = $true)][string]$SourceRoot,
  [Parameter(Mandatory = $true)][ValidateSet('baseline', 'candidate')][string]$Expectation,
  [Parameter(Mandatory = $true)][string]$EvidenceDir,
  [Parameter(Mandatory = $true)][string]$CargoTargetDir
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if (-not $IsWindows) { throw 'This proof requires native Windows.' }
$expectedSource = if ($Expectation -eq 'baseline') {
  '6aa2854f314481a459be1189b02c65a2450789ab'
} else {
  '63e8185eb184cf0af6b665f631e2314c7b14d9c6'
}
$SourceRoot = [IO.Path]::GetFullPath($SourceRoot)
$EvidenceDir = [IO.Path]::GetFullPath($EvidenceDir)
$CargoTargetDir = [IO.Path]::GetFullPath($CargoTargetDir)
$sourceSha = (& git -C $SourceRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $sourceSha -cne $expectedSource) { throw 'Unexpected updater source commit.' }
$sourceChanges = & git -C $SourceRoot status --porcelain
if ($LASTEXITCODE -ne 0 -or $sourceChanges) { throw 'Updater source checkout is not clean.' }
if ((Test-Path -LiteralPath $EvidenceDir) -and @(Get-ChildItem -LiteralPath $EvidenceDir -Force).Count -ne 0) {
  throw 'Evidence directory must be empty; stale receipts cannot qualify a run.'
}
New-Item -ItemType Directory -Force -Path $EvidenceDir | Out-Null
$manifest = Join-Path $PSScriptRoot 'Cargo.toml'
$lock = Join-Path $PSScriptRoot 'Cargo.lock'
$lockHash = (Get-FileHash -LiteralPath $lock -Algorithm SHA256).Hash
$pluginPath = (Join-Path $SourceRoot 'plugins/updater').Replace('\', '/')
$patch = 'patch.crates-io.tauri-plugin-updater.path=' + (ConvertTo-Json -Compress $pluginPath)
$env:CARGO_TARGET_DIR = $CargoTargetDir
$env:CARGO_INCREMENTAL = '0'
$env:CARGO_TERM_COLOR = 'never'
$env:UPDATER_INSTALL_PROOF_EVIDENCE = $EvidenceDir

& cargo +1.98.1 --config $patch test --locked --manifest-path $manifest --target x86_64-pc-windows-msvc --test windows_install -- --nocapture 2>&1 |
  Tee-Object -FilePath (Join-Path $EvidenceDir 'cargo.log')
$cargoExit = $LASTEXITCODE
if ((Get-FileHash -LiteralPath $lock -Algorithm SHA256).Hash -cne $lockHash) { throw 'Proof lockfile changed.' }
$afterSourceSha = (& git -C $SourceRoot rev-parse HEAD).Trim()
$sourceHeadExit = $LASTEXITCODE
$afterSourceChanges = & git -C $SourceRoot status --porcelain 2>&1
$sourceStatusExit = $LASTEXITCODE
$afterSourceChanges | Set-Content -LiteralPath (Join-Path $EvidenceDir 'source-status-after.txt')
& git -C $SourceRoot diff --no-ext-diff --binary HEAD -- 2>&1 |
  Set-Content -LiteralPath (Join-Path $EvidenceDir 'source-diff-after.patch')
$sourceDiffExit = $LASTEXITCODE
if ($sourceHeadExit -ne 0 -or $afterSourceSha -cne $sourceSha) { throw 'Updater source changed during proof.' }
if ($sourceStatusExit -ne 0 -or $afterSourceChanges) { throw 'Updater source changed during proof.' }
if ($sourceDiffExit -ne 0) { throw 'Could not capture updater source diff.' }
$failure = Get-Content -LiteralPath (Join-Path $EvidenceDir 'failed-msi-launch/outcome.json') -Raw | ConvertFrom-Json
$nsis = Get-Content -LiteralPath (Join-Path $EvidenceDir 'success-exe/outcome.json') -Raw | ConvertFrom-Json
$msi = Get-Content -LiteralPath (Join-Path $EvidenceDir 'success-msi/outcome.json') -Raw | ConvertFrom-Json
foreach ($success in @($nsis, $msi)) {
  if ($success.exitCode -ne 0 -or $success.cleanup.hookCalls -ne 1 -or -not $success.cleanup.resourceDropped) {
    throw 'Successful native launch did not preserve cleanup-and-exit behavior.'
  }
}
if ($failure.error.kind -cne 'io') { throw 'Failure fixture did not reach the native launch boundary.' }
if ($Expectation -eq 'baseline') {
  if ($cargoExit -ne 101 -or $failure.hookCalls -ne 1 -or -not $failure.resourceDropped) {
    throw 'Baseline did not reproduce the specific early-cleanup defect.'
  }
} elseif ($cargoExit -ne 0 -or $failure.hookCalls -ne 0 -or $failure.resourceDropped) {
  throw 'Candidate did not preserve application resources on native launch failure.'
}
@{
  sourceSha = $sourceSha
  expectation = $Expectation
  cargoExit = $cargoExit
  lockSha256 = $lockHash.ToLowerInvariant()
  rust = (& rustc +1.98.1 --version --verbose) -join "`n"
  platform = [Environment]::OSVersion.VersionString
  failure = $failure
  nsis = $nsis
  msi = $msi
} | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath (Join-Path $EvidenceDir 'summary.json')
Write-Host "Verified $Expectation source=$sourceSha cargo_exit=$cargoExit"
