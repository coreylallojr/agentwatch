# Install agentwatch on Windows (PowerShell 5.1+):
#   irm https://raw.githubusercontent.com/coreylallojr/agentwatch/main/install.ps1 | iex
# Downloads the latest release, checks its SHA-256, installs to %LOCALAPPDATA%\agentwatch\bin, adds that folder to
# your user PATH, then runs `agentwatch setup` (skills + MCP server for Claude Code).
# Set AGENTWATCH_NO_SETUP=1 to skip setup (for example when you use the Claude Code plugin instead).
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$repo = 'coreylallojr/agentwatch'
$asset = 'agentwatch-x86_64-pc-windows-msvc'
$base = "https://github.com/$repo/releases/latest/download"
$dir = Join-Path $env:LOCALAPPDATA 'agentwatch\bin'
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("agentwatch-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $dir, $tmp | Out-Null
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Write-Host "Downloading $asset.zip ..."
    Invoke-WebRequest "$base/$asset.zip" -OutFile "$tmp\$asset.zip" -UseBasicParsing
    Invoke-WebRequest "$base/SHA256SUMS" -OutFile "$tmp\SHA256SUMS" -UseBasicParsing
    $line = Get-Content "$tmp\SHA256SUMS" | Where-Object { $_ -match [regex]::Escape("$asset.zip") } | Select-Object -First 1
    if (-not $line) { throw "SHA256SUMS has no entry for $asset.zip" }
    $expected = ($line -split '\s+')[0].ToLower()
    $actual = (Get-FileHash "$tmp\$asset.zip" -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw "Checksum mismatch for $asset.zip (expected $expected, got $actual)" }
    Expand-Archive "$tmp\$asset.zip" $tmp -Force
    $exe = Join-Path $dir 'agentwatch.exe'
    if (Test-Path $exe) { & $exe stop | Out-Null }  # a running dashboard holds the file open
    Copy-Item "$tmp\$asset\agentwatch.exe" $exe -Force
    Write-Host "Installed $(& $exe --version) to $exe"
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (($userPath -split ';') -contains $dir)) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$dir").TrimStart(';'), 'User')
        Write-Host "Added $dir to your user PATH (open a new terminal to use 'agentwatch')."
    }
    $env:Path = "$env:Path;$dir"
    if (-not $env:AGENTWATCH_NO_SETUP) { & $exe setup } else { Write-Host "Skipped setup. Run 'agentwatch setup' later to add the skills and MCP server." }
} finally {
    Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
