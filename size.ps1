# Wrapper around `cargo build` that, once the build succeeds, prints the true
# (un-padded) size of every PE section of the produced binary.
#
# cargo's own build script (build/main.rs) runs before linking, so it cannot see
# the final binary — the section report has to happen after the build, hence this
# wrapper. `SizeOfRawData` is the file-aligned (512 B) size; we report the section
# header's `VirtualSize` instead, which is the real byte count with no rounding.
#
# All arguments are forwarded verbatim to cargo, so the toolchain selector works:
#     .\size.ps1                       # = cargo +nightly build --release
#     .\size.ps1 +nightly build        # debug build
#     .\size.ps1 build --release --features foo

param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $CargoArgs)

$ErrorActionPreference = 'Stop'

# The .cargo/config.toml pins this target triple for every profile.
$triple  = 'x86_64-pc-windows-msvc'
$binName = 'wordle_tui.exe'

if (-not $CargoArgs) { $CargoArgs = @('+nightly', 'build', '--release') }

cargo @CargoArgs
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$profile   = ($CargoArgs -contains '--release' -or $CargoArgs -contains '-r') ? 'release' : 'debug'
$targetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { 'target' }
$exe       = Join-Path $targetDir "$triple\$profile\$binName"
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
