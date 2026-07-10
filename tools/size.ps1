# Windows-only wrapper around `cargo build` that, once the build succeeds, prints the true
# (un-padded) size of every PE section of the produced wordle_tui.exe. It parses the PE header,
# so it targets the MSVC build exclusively.
#
# cargo's own build script (build/main.rs) runs before linking, so it cannot see
# the final binary — the section report has to happen after the build, hence this
# wrapper. `SizeOfRawData` is the file-aligned (512 B) size; we report the section
# header's `VirtualSize` instead, which is the real byte count with no rounding.
#
# All arguments are forwarded verbatim to cargo, so the toolchain selector works:
#     .\tools\size.ps1                       # = cargo +nightly build --release --target x86_64-pc-windows-msvc
#     .\tools\size.ps1 +nightly build        # debug build
#     .\tools\size.ps1 build --release --features foo

param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $CargoArgs)

$ErrorActionPreference = 'Stop'

# build-std needs an explicit --target, so the default build passes the Windows MSVC triple.
# Override by giving your own cargo args (e.g. a plain `build --release` with no --target).
$defaultTriple = 'x86_64-pc-windows-msvc'
$binName       = 'wordle_tui.exe'

if (-not $CargoArgs) { $CargoArgs = @('+nightly', 'build', '--release', '--target', $defaultTriple) }

cargo @CargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# An explicit `--target <triple>` nests output under target/<triple>/; without one cargo
# writes straight to target/<profile>/. Locate the binary accordingly.
$ti        = [array]::IndexOf($CargoArgs, '--target')
$triple    = if ($ti -ge 0 -and $ti + 1 -lt $CargoArgs.Count) { $CargoArgs[$ti + 1] } else { $null }
$profile   = ($CargoArgs -contains '--release' -or $CargoArgs -contains '-r') ? 'release' : 'debug'
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$exe       = if ($triple) { Join-Path $targetDir "$triple\$profile\$binName" } else { Join-Path $targetDir "$profile\$binName" }
if (-not (Test-Path $exe)) { throw "binary not found: $exe" }

$b       = [System.IO.File]::ReadAllBytes($exe)
$peOff   = [BitConverter]::ToInt32($b, 0x3C)
$numSec  = [BitConverter]::ToUInt16($b, $peOff + 6)
$optSize = [BitConverter]::ToUInt16($b, $peOff + 20)
$secTab  = $peOff + 24 + $optSize

$total = 0
$rows  = for ($i = 0; $i -lt $numSec; $i++) {
    $off    = $secTab + $i * 40
    $name   = [Text.Encoding]::ASCII.GetString($b, $off, 8).Trim([char]0)
    $vsize  = [BitConverter]::ToUInt32($b, $off + 8)
    $total += $vsize
    [pscustomobject]@{
        Section = $name
        Bytes   = '{0,9:N0}' -f $vsize
        KiB     = '{0,7:N2}' -f ($vsize / 1KB)
    }
}

Write-Host ("`n{0}  —  {1:N0} B on disk (file-aligned)" -f (Split-Path $exe -Leaf), $b.Length)
$rows | Format-Table -AutoSize | Out-String | Write-Host -NoNewline
Write-Host ("total VirtualSize: {0:N0} B  ({1:N2} KiB)`n" -f $total, ($total / 1KB))
