# Run the Catalyst Linux shell in a desktop window (the 320 x 170 panel at 4x).
#   .\scripts\run-sim.ps1                with a scripted demo robot
#   .\scripts\run-sim.ps1 -Robot 5805    against a real robot (team number, IP or host)
#   .\scripts\run-sim.ps1 -Light -Calm   light mode / the calm version
param(
    [string]$Robot,
    [switch]$Light,
    [switch]$Calm
)

$env:PATH = "$HOME\.cargo\bin;$env:PATH"
# Keep the saved robot address out of C:\etc on a development PC.
$env:CATALYST_CONF = Join-Path $env:LOCALAPPDATA "CatalystLinux\robot"

$shellArgs = @("--sim")
if ($Robot) { $shellArgs += @("--robot", $Robot) } else { $shellArgs += "--demo" }
if ($Light) { $shellArgs += "--light" }
if ($Calm) { $shellArgs += "--calm" }

Push-Location (Join-Path $PSScriptRoot "..\userland")
try {
    cargo run --release -p catalyst-shell -- @shellArgs
} finally {
    Pop-Location
}
