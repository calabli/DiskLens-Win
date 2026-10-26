#Requires -RunAsAdministrator
<#
.SYNOPSIS
  Re-attaches the DiskLens test VHDX (attachments do not survive a reboot).
#>
[CmdletBinding()]
param(
    [string]$Path = 'D:\DiskLensTest\ntfs-test.vhdx',
    [ValidatePattern('^[E-Z]$')][string]$DriveLetter = 'T'
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'TestVolume.Common.ps1')

if (-not (Test-Path -LiteralPath $Path)) { throw "$Path not found. Create it with New-TestVolume.ps1." }

Invoke-DiskPart @(
    "select vdisk file=`"$Path`""
    'attach vdisk'
)
# The partition keeps its drive letter in most cases; assign it explicitly if it did not come back.
if (-not (Test-Path "${DriveLetter}:\")) {
    $disk = Get-DiskImage -ImagePath $Path | Get-Disk
    $disk | Get-Partition | Where-Object Type -eq 'Basic' | Select-Object -First 1 |
        Set-Partition -NewDriveLetter $DriveLetter
}
Write-Host "Mounted $Path at ${DriveLetter}:\"
