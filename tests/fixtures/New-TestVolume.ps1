#Requires -RunAsAdministrator
<#
.SYNOPSIS
  Creates an NTFS test volume (VHDX) for DiskLens and fills it with edge cases.

.DESCRIPTION
  Creates an expandable VHDX, formats it as NTFS, mounts it at -DriveLetter and writes:
    basic\      files of known sizes
    hardlinks\  one file with two extra hard links (must be counted once)
    junction\   a junction and a symlink to a sibling folder (must not be followed)
    compressed\ an NTFS-compressed file (allocated size < logical size)
    sparse\     a sparse file (almost nothing allocated)
    ads\        a file with an alternate data stream
    longpath\   a path longer than 260 characters
    unicode\    CJK, emoji and combining-mark names
    empty\      empty folders and an empty file
    many\       -SmallFiles tiny files, 1000 per folder

  The script is ASCII-only on purpose: Windows PowerShell 5.1 reads BOM-less UTF-8
  scripts in the ANSI code page, so non-ASCII names are built from code points.

  Run in an elevated PowerShell. Use Mount-TestVolume.ps1 after a reboot and
  Remove-TestVolume.ps1 to detach or delete it.

.EXAMPLE
  .\New-TestVolume.ps1
  .\New-TestVolume.ps1 -SmallFiles 1000000 -SizeGB 16
#>
[CmdletBinding()]
param(
    [string]$Path = 'D:\DiskLensTest\ntfs-test.vhdx',
    [ValidateRange(1, 256)][int]$SizeGB = 8,
    [ValidatePattern('^[E-Z]$')][string]$DriveLetter = 'T',
    [ValidateRange(0, 5000000)][int]$SmallFiles = 100000
)

$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'TestVolume.Common.ps1')

if (Test-Path -LiteralPath $Path) {
    throw "$Path already exists. Mount it with Mount-TestVolume.ps1 or delete it with Remove-TestVolume.ps1 -DeleteFile."
}
if (Test-Path "${DriveLetter}:\") {
    throw "Drive ${DriveLetter}: is already in use. Pass a free letter with -DriveLetter."
}

New-Item -ItemType Directory -Force -Path (Split-Path -Parent $Path) | Out-Null
Write-Host "Creating $Path ($SizeGB GB, expandable) ..."
Invoke-DiskPart @(
    "create vdisk file=`"$Path`" maximum=$($SizeGB * 1024) type=expandable"
    "select vdisk file=`"$Path`""
    'attach vdisk'
    'create partition primary'
    'format fs=ntfs quick label=DLTEST'
    "assign letter=$DriveLetter"
)

$root = "${DriveLetter}:\"
function Join-Root([string]$rel) { Join-Path $root $rel }
function New-Dir([string]$rel) { New-Item -ItemType Directory -Force -Path (Join-Root $rel) | Out-Null; Join-Root $rel }
function New-SizedFile([string]$file, [long]$bytes) { & fsutil file createnew $file $bytes | Out-Null }

Write-Host 'basic ...'
$d = New-Dir 'basic'
New-SizedFile "$d\1KB.bin" 1024
New-SizedFile "$d\1MB.bin" 1048576
New-SizedFile "$d\64MB.bin" 67108864
Set-Content -LiteralPath "$d\hello.txt" -Value 'hello' -NoNewline -Encoding ASCII

Write-Host 'hardlinks ...'
$d = New-Dir 'hardlinks'
New-SizedFile "$d\original.bin" 10485760
New-Item -ItemType HardLink -Path "$d\link1.bin" -Target "$d\original.bin" | Out-Null
$sub = New-Dir 'hardlinks\sub'
New-Item -ItemType HardLink -Path "$sub\link2.bin" -Target "$d\original.bin" | Out-Null

Write-Host 'junction and symlink ...'
$target = New-Dir 'junction\target'
New-SizedFile "$target\data.bin" 5242880
New-Item -ItemType Junction -Path (Join-Root 'junction\to-target') -Target $target | Out-Null
New-Item -ItemType SymbolicLink -Path (Join-Root 'junction\symlink-to-target') -Target $target | Out-Null
New-Item -ItemType Junction -Path (Join-Root 'junction\target\loop-to-root') -Target (Join-Root 'junction') | Out-Null

Write-Host 'compressed ...'
$d = New-Dir 'compressed'
$line = ('DiskLens compressible test line 0123456789 ' * 4) + "`r`n"
$sb = New-Object System.Text.StringBuilder
for ($i = 0; $i -lt 120000; $i++) { [void]$sb.Append($line) }
[System.IO.File]::WriteAllText("$d\text.log", $sb.ToString(), [System.Text.Encoding]::ASCII)
& compact /c /q "$d\text.log" | Out-Null

Write-Host 'sparse ...'
$d = New-Dir 'sparse'
New-SizedFile "$d\sparse.bin" 104857600
& fsutil sparse setflag "$d\sparse.bin" | Out-Null
& fsutil sparse setrange "$d\sparse.bin" 0 104857600 | Out-Null

Write-Host 'ads ...'
$d = New-Dir 'ads'
Set-Content -LiteralPath "$d\with-stream.txt" -Value 'main stream' -Encoding ASCII
Set-Content -LiteralPath "$d\with-stream.txt" -Stream 'extra' -Value ('x' * 1048576) -Encoding ASCII

Write-Host 'longpath ...'
$segment = 'segment-' + ('L' * 40)
$long = Join-Root 'longpath'
for ($i = 0; $i -lt 8; $i++) { $long = "$long\$segment-$i" }
[System.IO.Directory]::CreateDirectory("\\?\$long") | Out-Null
[System.IO.File]::WriteAllBytes("\\?\$long\deep.bin", (New-Object byte[] 4096))

Write-Host 'unicode ...'
$d = New-Dir 'unicode'
$cjk = -join ([char[]](0x6D4B, 0x8BD5, 0x6587, 0x4EF6))          # CJK "test file"
$emoji = [char]::ConvertFromUtf32(0x1F4C1) + '-folder'            # folder emoji
$combining = 'e' + [char]0x0301 + '-combining'                   # e + combining acute
foreach ($name in @("$cjk.txt", "$emoji.txt", "$combining.txt")) {
    [System.IO.File]::WriteAllBytes((Join-Path $d $name), (New-Object byte[] 2048))
}
New-Item -ItemType Directory -Path (Join-Path $d $cjk) | Out-Null

Write-Host 'empty ...'
New-Dir 'empty\a' | Out-Null
New-Dir 'empty\b\c' | Out-Null
New-Item -ItemType File -Path (Join-Root 'empty\zero.txt') | Out-Null

if ($SmallFiles -gt 0) {
    Write-Host "many ($SmallFiles small files) ..."
    $one = [byte[]](0x41)
    for ($i = 0; $i -lt $SmallFiles; $i++) {
        if ($i % 1000 -eq 0) {
            $dir = New-Dir ('many\{0:D5}' -f [int]($i / 1000))
            Write-Progress -Activity 'Small files' -Status "$i / $SmallFiles" -PercentComplete ($i * 100 / $SmallFiles)
        }
        [System.IO.File]::WriteAllBytes("$dir\f$i.dat", $one)
    }
    Write-Progress -Activity 'Small files' -Completed
}

Write-Host ''
Write-Host "Done. Test volume mounted at ${DriveLetter}:\ from $Path"
