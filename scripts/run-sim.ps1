# Run the Catalyst Linux shell in a desktop window (the 320 x 170 panel at 4x).
#
#   .\scripts\run-sim.ps1                    a demo robot, at the handheld's estimated speed
#   .\scripts\run-sim.ps1 -Speed worse       slower than the handheld: does it still hold up?
#   .\scripts\run-sim.ps1 -Speed pc          as fast as this PC goes
#   .\scripts\run-sim.ps1 -Robot 5805        a real robot (team number, IP or host)
#   .\scripts\run-sim.ps1 -Light -Accent blue -Text large
#
# The window title shows, once a second, the frames drawn, what a frame costs
# and how busy the handheld's processor would be. The speeds are estimates
# until a CardputerZero has been measured with "catalyst-shell --bench".
param(
    [string]$Robot,
    [ValidateSet("device", "worse", "pc")][string]$Speed = "device",
    [ValidateSet("orange", "blue", "green", "violet", "rose")][string]$Accent,
    [ValidateSet("compact", "standard", "large")][string]$Text,
    [switch]$Light,
    [switch]$Calm,
    [switch]$Perf
)

$env:PATH = "$HOME\.cargo\bin;$env:PATH"
# Keep the saved robot address, settings and logs out of C:\etc and C:\data on a development PC.
$state = Join-Path $env:LOCALAPPDATA "CatalystLinux"
$env:CATALYST_CONF = Join-Path $state "robot"
$env:CATALYST_LOGS = Join-Path $state "logs"

$shellArgs = @("--sim", "--speed", $Speed)
if ($Robot) { $shellArgs += @("--robot", $Robot) } else { $shellArgs += "--demo" }
if ($Accent) { $shellArgs += @("--accent", $Accent) }
if ($Text) { $shellArgs += @("--text", $Text) }
if ($Light) { $shellArgs += "--light" }
if ($Calm) { $shellArgs += "--calm" }
if ($Perf) { $shellArgs += "--perf" }

Push-Location (Join-Path $PSScriptRoot "..\userland")
try {
    cargo run --release -p catalyst-shell -- @shellArgs
} finally {
    Pop-Location
}
