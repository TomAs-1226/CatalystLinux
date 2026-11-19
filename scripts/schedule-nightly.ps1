# Build and publish a nightly Catalyst Linux every night, from this PC.
#
#   .\scripts\schedule-nightly.ps1              every night at 02:30
#   .\scripts\schedule-nightly.ps1 -At 04:00    at another time
#   .\scripts\schedule-nightly.ps1 -Remove      stop
#
# It registers a Windows scheduled task that runs scripts/release.sh nightly
# --publish inside WSL. The PC must be on (it may be asleep: the task wakes
# it) and the GitHub CLI signed in inside WSL. Each night's log is kept in
# ~/catalyst-linux-build/nightly.log in WSL.
param(
    [string]$At = "02:30",
    [switch]$Remove
)

$name = "Catalyst Linux nightly"
if ($Remove) {
    Unregister-ScheduledTask -TaskName $name -Confirm:$false
    Write-Output "removed: $name"
    return
}

$repo = (Resolve-Path "$PSScriptRoot\..").Path
# The repository as WSL sees it: C:\Users\... becomes /mnt/c/Users/...
$wslPath = "/mnt/" + $repo.Substring(0, 1).ToLower() + $repo.Substring(2).Replace("\", "/")
$command = "cd '$wslPath' && bash scripts/release.sh nightly --publish >> ~/catalyst-linux-build/nightly.log 2>&1"
$action = New-ScheduledTaskAction -Execute "wsl.exe" -Argument "-e bash -lc `"$command`""
$trigger = New-ScheduledTaskTrigger -Daily -At $At
$settings = New-ScheduledTaskSettingsSet -WakeToRun -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Hours 4)
Register-ScheduledTask -TaskName $name -Action $action -Trigger $trigger -Settings $settings -Description "Builds Catalyst Linux and publishes it as a nightly release." -Force | Out-Null
Write-Output "registered: $name, every night at $At"
