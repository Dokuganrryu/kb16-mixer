<#
.SYNOPSIS
Writes the KB16 Mixer firmware into a DOIO KB16-01 rev2 pad that is in its bootloader.

.DESCRIPTION
The script writes only the firmware area (DFU alternate setting 2, address 0x08002000).
It never writes the bootloader. It refuses to write unless:
  - exactly one device with the USB ID 1EAF:0003 (the bootloader) is connected,
  - that device has alternate setting 2 and no other flash address is named for it,
  - the image is not empty, is not larger than 0x1D400 bytes, and starts with a
    stack pointer in RAM and a reset vector in the firmware area,
  - you type YES.
At the end, unplug the pad and plug it in again.

Works in Windows PowerShell 5.1 and in PowerShell 7.
See docs/install.md for the full procedure.

.PARAMETER Firmware
The image to write. Default: firmware\kb16-mixer-firmware.bin next to this script,
then ..\firmware\target\kb16-mixer-firmware.bin (a build from source).

.PARAMETER DfuUtil
The path of dfu-util. Default: dfu-util.exe or dfu-util-static.exe on the PATH, then
the same names in the tools folder next to this script.

.PARAMETER Backup
Read the present firmware area into this new file before the write.
The script does not overwrite a file that exists.

.PARAMETER DryRun
Do all the checks (and the backup, if you ask for it), but do not write the pad,
do not stop the app and do not restart the pad.

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\flash.ps1 -Backup .\stock-backup.bin

.EXAMPLE
powershell -ExecutionPolicy Bypass -File .\flash.ps1 -DryRun
#>
[CmdletBinding()]
param(
    [string]$Firmware,
    [string]$DfuUtil,
    [string]$Backup,
    [switch]$DryRun
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

$UsbId = '1eaf:0003'
$AltSetting = 2
$AppStart = [uint32]0x08002000
# The name page starts at 0x0801F400; a larger image would overwrite it and the crash log.
$MaxImage = 0x1D400
$RamStart = [uint32]0x20000000
$RamEnd = [uint32]0x20005000
# The size that a read of alternate setting 2 gave on the tested pad (0x08002000 to 0x08020000).
$FullAreaSize = 122880
$WaitSeconds = 120

function Stop-Flash([string]$Message) {
    Write-Host ''
    Write-Host "REFUSED: $Message" -ForegroundColor Red
    Write-Host 'Nothing was written to the pad.'
    exit 1
}

function Find-DfuUtil {
    if ($DfuUtil) {
        if (Test-Path -LiteralPath $DfuUtil -PathType Leaf) { return (Resolve-Path -LiteralPath $DfuUtil).Path }
        Stop-Flash "dfu-util not found at $DfuUtil."
    }
    # The dfu-util release has a static build (one file) and a build that needs libusb-1.0.dll.
    foreach ($name in @('dfu-util.exe', 'dfu-util-static.exe')) {
        $onPath = Get-Command $name -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if ($onPath) { return $onPath.Source }
    }
    foreach ($name in @('dfu-util.exe', 'dfu-util-static.exe')) {
        $local = Join-Path $PSScriptRoot "tools\$name"
        if (Test-Path -LiteralPath $local -PathType Leaf) { return $local }
    }
    $local = Join-Path $PSScriptRoot 'tools'
    Stop-Flash ("dfu-util not found. Put dfu-util-static.exe in $local, put it on the PATH, or give -DfuUtil <path>. " +
        'Get it from https://dfu-util.sourceforge.net (see docs/install.md).')
}

function Find-Firmware {
    if ($Firmware) {
        if (Test-Path -LiteralPath $Firmware -PathType Leaf) { return (Resolve-Path -LiteralPath $Firmware).Path }
        Stop-Flash "No firmware image at $Firmware."
    }
    foreach ($candidate in @(
            (Join-Path $PSScriptRoot 'firmware\kb16-mixer-firmware.bin'),
            (Join-Path $PSScriptRoot '..\firmware\target\kb16-mixer-firmware.bin'))) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return (Resolve-Path -LiteralPath $candidate).Path }
    }
    Stop-Flash 'No firmware image found. Give -Firmware <path>.'
}

# Native programs write progress and errors to stderr. In Windows PowerShell 5.1, "2>&1" turns
# each stderr line into an error record, which 'Stop' would throw on, so the call runs under
# 'Continue' and every line becomes plain text.
function Invoke-Dfu([string[]]$Arguments, [switch]$Show) {
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = @(& $script:Dfu @Arguments 2>&1 | ForEach-Object {
                $text = "$_"
                if ($Show) { Write-Host $text }
                $text
            })
        return [pscustomobject]@{ Code = $LASTEXITCODE; Lines = $lines }
    }
    finally { $ErrorActionPreference = $previous }
}

function Test-Image([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -eq 0) { Stop-Flash "The image $Path is empty." }
    if ($bytes.Length -lt 8) { Stop-Flash "The image $Path is too small ($($bytes.Length) bytes) to be firmware." }
    if ($bytes.Length -gt $MaxImage) {
        Stop-Flash ("The image is $($bytes.Length) bytes. The limit is $MaxImage bytes (0x1D400). " +
            'A larger image overwrites the name page and the crash log. A full backup of the stock firmware is ' +
            'larger than this limit: restore it with dfu-util, see docs/stock-firmware.md.')
    }
    $sp = [BitConverter]::ToUInt32($bytes, 0)
    $reset = [BitConverter]::ToUInt32($bytes, 4)
    if (-not ($sp -gt $RamStart -and $sp -le $RamEnd)) {
        Stop-Flash ('The first word ({0}) is not a stack pointer in RAM (0x20000000 to 0x20005000). This is not a firmware image for this pad.' -f ('0x{0:X8}' -f $sp))
    }
    if (-not ($reset -gt $AppStart -and $reset -lt ($AppStart + $MaxImage) -and ($reset -band 1) -eq 1)) {
        Stop-Flash ('The second word ({0}) is not a reset vector in the firmware area (0x08002000 to 0x0801F400). The image is not linked for this pad.' -f ('0x{0:X8}' -f $reset))
    }
    return [pscustomobject]@{ Size = $bytes.Length; StackPointer = $sp; Reset = $reset }
}

# One "Found DFU" line for each alternate setting of each device.
function Get-Bootloaders {
    $result = Invoke-Dfu @('-l')
    $found = @()
    foreach ($line in $result.Lines) {
        if ($line -match ('Found DFU: \[' + $UsbId + '\].*?path="([^"]*)".*?alt=(\d+), name="([^"]*)"')) {
            $found += [pscustomobject]@{ Path = $Matches[1]; Alt = [int]$Matches[2]; Name = $Matches[3] }
        }
    }
    return , $found
}

function Assert-OneBootloader($Found) {
    $paths = @($Found | ForEach-Object { $_.Path } | Sort-Object -Unique)
    if ($paths.Count -gt 1) {
        Stop-Flash "$($paths.Count) devices with the USB ID 1EAF:0003 are connected. Connect only the pad that you want to flash."
    }
    $alt = @($Found | Where-Object { $_.Alt -eq $AltSetting })
    if ($alt.Count -ne 1) {
        Stop-Flash 'The bootloader has no alternate setting 2. This is not the bootloader of a KB16-01 rev2.'
    }
    $name = $alt[0].Name
    # The stm32duino bootloader names the flash address in the name of each alternate setting.
    if ($name -match '0x0?8[0-9A-Fa-f]{6}' -and $name -notmatch '0x0?8002000') {
        Stop-Flash "Alternate setting 2 writes to a different address ($name). This is not the expected bootloader."
    }
    if ($name -notmatch '0x0?8002000') {
        Write-Host "WARNING: alternate setting 2 does not name its address (name: `"$name`")." -ForegroundColor Yellow
    }
    return $name
}

function Find-App {
    foreach ($candidate in @(
            (Join-Path $PSScriptRoot 'kb16-mixer.exe'),
            (Join-Path $PSScriptRoot '..\app\target\release\kb16-mixer.exe'))) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return (Resolve-Path -LiteralPath $candidate).Path }
    }
    return $null
}

function Write-Next {
    Write-Host ''
    Write-Host 'NEXT: unplug the pad and plug it in again. The bootloader starts the new firmware only after that.' -ForegroundColor Cyan
}

# --- Checks that do not need the pad ---------------------------------------------------------

$script:Dfu = Find-DfuUtil
$image = Find-Firmware
$info = Test-Image $image
$hash = (Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash.ToLowerInvariant()

Write-Host "firmware: $image"
Write-Host ("          {0} bytes, stack pointer 0x{1:X8}, reset vector 0x{2:X8}: OK" -f $info.Size, $info.StackPointer, $info.Reset)
Write-Host "  SHA-256: $hash"
Write-Host "dfu-util: $script:Dfu"
if ($DryRun) { Write-Host 'DRY RUN: the script does not write the pad, stop the app or restart the pad.' -ForegroundColor Yellow }
if ($Backup -and (Test-Path -LiteralPath $Backup)) {
    Stop-Flash "The backup file $Backup exists. Give a new file name."
}

# --- Find the bootloader ---------------------------------------------------------------------

$restart = @()
$found = Get-Bootloaders
if ($found.Count -eq 0) {
    if ($DryRun) {
        Write-Host ''
        Write-Host 'No pad in bootloader mode (USB 1EAF:0003) is connected.'
        Write-Host 'A real run asks to restart the pad into its bootloader, or waits for you to do it.'
        Write-Host 'DRY RUN complete: the image and dfu-util are OK. Nothing was written.'
        exit 0
    }
    $app = Find-App
    $asked = $false
    if ($app) {
        $answer = Read-Host 'The pad is not in bootloader mode. Restart it into its bootloader with kb16-mixer.exe? (y/N)'
        if ($answer -match '^(y|yes)$') {
            $asked = $true
            # A running app keeps the pad open and talks to it, so it stops before the restart
            # and starts again from the same file at the end.
            $restart = @(Get-Process -Name kb16-mixer -ErrorAction SilentlyContinue |
                    ForEach-Object { $_.Path } | Where-Object { $_ } | Sort-Object -Unique)
            Get-Process -Name kb16-mixer -ErrorAction SilentlyContinue | Stop-Process -Force
            # The app is a GUI program: "&" does not wait for it and does not set $LASTEXITCODE.
            $request = Start-Process -FilePath $app -ArgumentList '--bootloader' -Wait -PassThru -WindowStyle Hidden
            if ($request.ExitCode -eq 0) { Write-Host 'The pad was told to enter its bootloader.' }
            else { Write-Host 'The pad did not answer. It possibly has other firmware.' -ForegroundColor Yellow; $asked = $false }
        }
    }
    if (-not $asked) {
        Write-Host 'Put the pad into its bootloader now:'
        Write-Host '  1. Unplug the pad.'
        Write-Host '  2. Hold the top-left key.'
        Write-Host '  3. Plug the pad in. Hold the key for 2 more seconds.'
        Write-Host '  From the stock firmware, see docs/stock-firmware.md for other ways.'
    }
    Write-Host "Waiting up to $WaitSeconds seconds for the bootloader (USB 1EAF:0003)..."
    $deadline = (Get-Date).AddSeconds($WaitSeconds)
    while ($found.Count -eq 0 -and (Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 1
        $found = Get-Bootloaders
    }
    if ($found.Count -eq 0) {
        Stop-Flash ('No pad in bootloader mode (USB 1EAF:0003) was found. If Windows shows "Maple 003" with a ' +
            'warning sign, install the WinUSB driver (docs/install.md). Then run the script again.')
    }
    # A device that has just come up can list before all its alternate settings are ready.
    Start-Sleep -Seconds 1
    $found = Get-Bootloaders
}
$altName = Assert-OneBootloader $found
Write-Host "bootloader: USB 1EAF:0003, alternate setting 2 (`"$altName`")"

# --- Backup ----------------------------------------------------------------------------------

if ($Backup) {
    Write-Host ''
    Write-Host "Reading the firmware area into $Backup ..."
    $read = Invoke-Dfu @('-d', $UsbId, '-a', "$AltSetting", '-U', $Backup)
    if (-not (Test-Path -LiteralPath $Backup -PathType Leaf)) {
        $read.Lines | Select-Object -Last 5 | ForEach-Object { Write-Host $_ }
        Stop-Flash 'dfu-util wrote no backup file.'
    }
    $copy = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Backup).Path)
    if ($copy.Length -eq 0) { Stop-Flash 'The backup file is empty. Delete it: it is not a backup.' }
    $blank = $true
    foreach ($b in $copy) { if ($b -ne 0xFF) { $blank = $false; break } }
    if ($blank) { Stop-Flash 'The backup holds only 0xFF bytes (an empty flash area). Delete it: it is not a backup. Ask for help in an issue.' }
    $backupHash = (Get-FileHash -LiteralPath $Backup -Algorithm SHA256).Hash.ToLowerInvariant()
    Write-Host "backup: $($copy.Length) bytes, SHA-256 $backupHash"
    if ($copy.Length -ne $FullAreaSize) {
        # A short read is possibly not the whole stock firmware: keep the pad as it is.
        Stop-Flash ("The backup has $($copy.Length) bytes, but the tested pad gave $FullAreaSize bytes. " +
            'Keep the file, but do not trust it as a full copy. Ask for help in an issue.')
    }
    Write-Host 'Keep the backup file and its SHA-256 in a safe place.'
}

# --- Write -----------------------------------------------------------------------------------

if ($DryRun) {
    Write-Host ''
    Write-Host 'DRY RUN complete: all checks passed. Nothing was written.'
    exit 0
}

Write-Host ''
Write-Host 'Do not unplug the pad while the write runs (about 10 seconds).' -ForegroundColor Yellow
$confirm = Read-Host 'Type YES to write this image into the pad'
if ($confirm -cne 'YES') { Stop-Flash 'You did not type YES.' }

$write = Invoke-Dfu @('-d', $UsbId, '-a', "$AltSetting", '-D', $image, '-R') -Show
$done = @($write.Lines | Where-Object { $_ -match 'Download done' }).Count -gt 0
# dfu-util can exit with 74 after "Download done": the bootloader does not answer the reset request.
Write-Host "dfu-util exit $($write.Code) (74 after 'Download done' is normal)"

foreach ($exe in $restart) {
    Start-Process -FilePath $exe
    Write-Host "Started the app again: $exe"
}

if (-not $done) {
    Write-Host ''
    Write-Host 'FAILED: dfu-util did not show "Download done". Do not unplug the pad yet.' -ForegroundColor Red
    Write-Host 'Run the script again. If it fails again, see docs/troubleshooting.md.'
    exit 1
}
Write-Host 'Download done.' -ForegroundColor Green
Write-Next
exit 0
