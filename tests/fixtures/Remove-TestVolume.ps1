#Requires -RunAsAdministrator
<#
.SYNOPSIS
  Detaches the DiskLens test VHDX, and deletes the file when -DeleteFile is given.
#>
[CmdletBinding()]
param(
    [string]$Path = 'D:\DiskLensTest\ntfs-test.vhdx',
    [switch]$DeleteFile
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'TestVolume.Common.ps1')

if (-not (Test-Path -LiteralPath $Path)) { throw "$Path not found." }

$attached = (Get-DiskImage -ImagePath $Path).Attached
if ($attached) {
    Invoke-DiskPart @(
        "select vdisk file=`"$Path`""
        'detach vdisk'
    )
    Write-Host "Detached $Path"
}

if ($DeleteFile) {
    Remove-Item -LiteralPath $Path
    Write-Host "Deleted $Path"
}
