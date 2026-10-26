# Shared helpers for the DiskLens test volume scripts. ASCII-only, see New-TestVolume.ps1.

function Invoke-DiskPart([string[]]$Lines) {
    $script = Join-Path $env:TEMP ("dl-diskpart-{0}.txt" -f [guid]::NewGuid())
    Set-Content -LiteralPath $script -Value $Lines -Encoding ASCII
    try {
        $output = & diskpart /s $script
        if ($LASTEXITCODE -ne 0) {
            throw "diskpart failed (exit $LASTEXITCODE):`n$($output -join "`n")"
        }
    }
    finally {
        Remove-Item -LiteralPath $script -ErrorAction SilentlyContinue
    }
}
