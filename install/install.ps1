<#
.SYNOPSIS
    HooCode one-click installer for Windows.

.DESCRIPTION
    irm https://kolisachint.github.io/hoocode/install.ps1 | iex

    What it does:
      1. Works out your architecture and picks the matching standalone build.
      2. Downloads it from the latest GitHub release and verifies its SHA256
         against the release's checksums.txt.
      3. Unpacks it into %USERPROFILE%\.hoocode\lib\hoocode and puts
         hoocode.cmd / hoo.cmd in %USERPROFILE%\.hoocode\bin.
      4. Optionally pre-seeds the external Rust tools (fd, rg, embsearch,
         webtools, voicetools) into that same bin directory, which is exactly
         where HooCode looks for them - so the first run is fast and works with
         no network.
      5. Adds the bin directory to your user PATH.

    Nothing here needs administrator rights: everything lands under your own
    profile and only the *user* PATH is touched.

.PARAMETER Version
    Release tag to install. Defaults to the latest release. "1.2.3" and
    "v1.2.3" are both accepted.

.PARAMETER InstallDir
    Install root. Defaults to $env:USERPROFILE\.hoocode.

.PARAMETER Tools
    Which external tools to pre-seed. Defaults to all of them. Use -NoTools to
    skip the step entirely.

.PARAMETER NoTools
    Skip the external tools. HooCode downloads what it needs on first use.

.PARAMETER NoModifyPath
    Do not touch the user PATH.

.NOTES
    Environment only: HOOCODE_RELEASE_BASE_URL fetches release archives from
    somewhere other than GitHub, expecting <base>/<tag>/hoocode-<target>.zip and
    <base>/<tag>/checksums.txt. For mirrors, air-gapped installs, and tests.

.EXAMPLE
    irm https://kolisachint.github.io/hoocode/install.ps1 | iex

.EXAMPLE
    # With options, the download has to be saved first - a piped script cannot
    # take arguments.
    irm https://kolisachint.github.io/hoocode/install.ps1 -OutFile install.ps1
    .\install.ps1 -NoTools
#>
[CmdletBinding()]
param(
    [string] $Version      = $(if ($env:HOOCODE_VERSION) { $env:HOOCODE_VERSION } else { 'latest' }),
    [string] $InstallDir   = $(if ($env:HOOCODE_INSTALL_DIR) { $env:HOOCODE_INSTALL_DIR } else { Join-Path $env:USERPROFILE '.hoocode' }),
    [string[]] $Tools      = @('fd', 'rg', 'embsearch', 'webtools', 'voicetools'),
    [switch] $NoTools,
    [switch] $NoModifyPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$Repo    = 'kolisachint/hoocode'
$Website = 'https://kolisachint.github.io/hoocode/'

# ------------------------------------------------------------------ output --
function Write-Step { param([string] $Message) Write-Host "==> " -ForegroundColor Cyan -NoNewline; Write-Host $Message }
function Write-Note { param([string] $Message) Write-Host "    $Message" -ForegroundColor DarkGray }
function Write-Ok   { param([string] $Message) Write-Host "    $Message" -ForegroundColor Green }
function Write-Warn { param([string] $Message) Write-Host "warning: " -ForegroundColor Yellow -NoNewline; Write-Host $Message }
# `exit` runs in the caller's scope under `irm | iex`, so it does not end the
# install - it ends the window the install was typed into. A terminating error
# stops the script and leaves the shell standing.
function Fail       { param([string] $Message) Write-Host "error: " -ForegroundColor Red -NoNewline; Write-Host $Message; throw 'hoocode: install aborted' }

# TLS 1.2 is not the default on Windows PowerShell 5.1, and GitHub serves
# nothing older. Without this the very first download fails with an unhelpful
# "could not create SSL/TLS secure channel".
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
} catch { }

# PowerShell 5.1's Invoke-WebRequest renders a progress bar per chunk, which on a
# 60MB download costs more wall-clock time than the download itself.
$PrevProgress = $ProgressPreference
$ProgressPreference = 'SilentlyContinue'

# -------------------------------------------------------------- platform ----
$Arch = switch ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture) {
    'X64'   { 'x64' }
    'Arm64' { 'arm64' }
    default { $null }
}
if (-not $Arch) {
    Fail "unsupported CPU architecture. Install from npm instead (needs Node >= 20):`n    npm install -g @kolisachint/hoocode-agent"
}

# Bun has no windows-arm64 compile target, so there is no arm64 archive to
# download. Windows on ARM runs x64 binaries under emulation, which is a working
# HooCode rather than no HooCode - so ship the x64 build and say so once.
$Target = 'windows-x64'
if ($Arch -eq 'arm64') {
    Write-Step 'Platform: windows-arm64'
    Write-Note 'no native arm64 build yet - installing the x64 build, which Windows runs under emulation'
} else {
    Write-Step "Platform: $Target"
}

# --------------------------------------------------------------- version ----
if ($Version -eq 'latest') {
    Write-Step 'Resolving the latest release...'
    $Tag = $null
    # Ask the API, but do not require it: the unauthenticated limit is 60 calls an
    # hour per IP, which a shared office address can exhaust without the person at
    # the keyboard ever having run this before. A blocked api.github.com does the
    # same. install.sh has fallen back for this reason since day one; this is the
    # same fallback.
    try {
        $Release = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases/latest" -Headers @{ 'User-Agent' = 'hoocode-installer' }
        $Tag = $Release.tag_name
    } catch {
        $ApiError = $_.Exception.Message
    }

    # /releases/latest is a plain redirect to /releases/tag/<tag>. It costs no API
    # quota and lives on github.com, which a network that allows the download of
    # the release archive already has to allow.
    if (-not $Tag) {
        try {
            $Probe = Invoke-WebRequest -Uri "https://github.com/$Repo/releases/latest" -UseBasicParsing -Headers @{ 'User-Agent' = 'hoocode-installer' }
            $Final = $null
            # PowerShell 7 hangs the resolved URL off the request message, 5.1 off
            # the response. Ask for both rather than branch on the version.
            try { $Final = $Probe.BaseResponse.RequestMessage.RequestUri.AbsoluteUri } catch { }
            if (-not $Final) { try { $Final = $Probe.BaseResponse.ResponseUri.AbsoluteUri } catch { } }
            if ($Final -and ($Final -match '/releases/tag/(.+)$')) { $Tag = $Matches[1] }
        } catch { }
    }

    if (-not $Tag) {
        # Say what actually went wrong - "could not reach the GitHub API" covers a
        # rate limit, a proxy, and a DNS failure alike, and sends people hunting
        # the wrong one. And -Version is not advice a piped install can take: you
        # cannot pass arguments through `iex`, so name the variable that works.
        if (-not $ApiError) { $ApiError = 'no error reported' }
        Fail @"
could not resolve the latest release.
    GitHub API: $ApiError
    Fell back to https://github.com/$Repo/releases/latest, which also failed.

    Install a known version instead - set it in the environment, because a
    piped script cannot take arguments:
      `$env:HOOCODE_VERSION = 'v0.5.80'
      irm $Website`install.ps1 | iex
"@
    }
} else {
    $Tag = if ($Version.StartsWith('v')) { $Version } else { "v$Version" }
}
Write-Note "version $Tag"

$Asset = "hoocode-$Target.zip"
# A mirror serves the same layout under its own root; unset, this is GitHub.
# Matches HOOCODE_RELEASE_BASE_URL in install.sh — for mirrors, air-gapped
# installs, and the installers' own tests.
$Base = if ($env:HOOCODE_RELEASE_BASE_URL) {
    "$($env:HOOCODE_RELEASE_BASE_URL.TrimEnd('/'))/$Tag"
} else {
    "https://github.com/$Repo/releases/download/$Tag"
}

# ------------------------------------------------------------------ paths ---
$LibDir = Join-Path $InstallDir 'lib\hoocode'
$BinDir = Join-Path $InstallDir 'bin'
$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("hoocode-install-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

try {
    # ---------------------------------------------------------- download ----
    Write-Step "Downloading $Asset..."
    $ArchivePath = Join-Path $TmpDir $Asset
    try {
        Invoke-WebRequest -Uri "$Base/$Asset" -OutFile $ArchivePath -UseBasicParsing
    } catch {
        Fail @"
no build for $Target in $Tag.
    Checked: $Base/$Asset
    Install from npm instead (works anywhere Node >= 20 runs):
      npm install -g @kolisachint/hoocode-agent
"@
    }

    # Checksum: TLS already authenticated the host, so this is about catching a
    # truncated or tampered artifact, and it is cheap enough to always do.
    try {
        $ChecksumPath = Join-Path $TmpDir 'checksums.txt'
        Invoke-WebRequest -Uri "$Base/checksums.txt" -OutFile $ChecksumPath -UseBasicParsing
        # .NET regex has no \Q...\E, so escape the literal explicitly.
        $AssetRe = [regex]::Escape($Asset)
        $Line = Get-Content $ChecksumPath | Where-Object { $_ -match "\s$AssetRe$" } | Select-Object -First 1
        if ($Line) {
            $Expected = ($Line -split '\s+')[0]
            $Actual   = (Get-FileHash -Path $ArchivePath -Algorithm SHA256).Hash.ToLower()
            if ($Expected.ToLower() -ne $Actual) {
                Fail @"
checksum mismatch for $Asset.
    expected $Expected
    actual   $Actual
    Refusing to install. Re-run, and if it persists open an issue:
      https://github.com/$Repo/issues
"@
            }
            Write-Ok 'checksum verified'
        } else {
            Write-Warn "no checksum listed for $Asset; skipping verification."
        }
    } catch {
        Write-Warn 'could not fetch checksums.txt; skipping verification.'
    }

    # ----------------------------------------------------------- install ----
    Write-Step "Installing into $LibDir..."
    $Stage = Join-Path $TmpDir 'stage'
    Expand-Archive -Path $ArchivePath -DestinationPath $Stage -Force

    $Exe = Join-Path $Stage 'hoocode.exe'
    if (-not (Test-Path $Exe)) {
        Fail "the archive did not contain hoocode.exe. Please report this: https://github.com/$Repo/issues"
    }

    # Windows will not overwrite a running executable, and the single most likely
    # reason to run this script is upgrading while a session is open. Say that,
    # rather than letting it fail as an opaque access-denied.
    if (Test-Path $LibDir) {
        try {
            Remove-Item -Recurse -Force $LibDir
        } catch {
            Fail "could not replace the existing install at $LibDir.`n    Close any running hoocode session and re-run."
        }
    }
    New-Item -ItemType Directory -Path (Split-Path $LibDir -Parent) -Force | Out-Null
    Move-Item -Path $Stage -Destination $LibDir
    New-Item -ItemType Directory -Path $BinDir -Force | Out-Null

    # Shims rather than symlinks: creating a symlink on Windows needs either
    # Developer Mode or elevation, and this installer needs neither. `%*`
    # forwards arguments; `@echo off` keeps the shim out of the output.
    foreach ($name in @('hoocode', 'hoo')) {
        $shim = Join-Path $BinDir "$name.cmd"
        "@echo off`r`n`"$LibDir\hoocode.exe`" %*" | Set-Content -Path $shim -Encoding ASCII
    }

    # Mark-of-the-web: anything downloaded carries a zone identifier that makes
    # SmartScreen interrupt the first run. Clearing it is what the user would be
    # told to do anyway.
    Get-ChildItem -Path $LibDir -Recurse -File -ErrorAction SilentlyContinue |
        ForEach-Object { Unblock-File -Path $_.FullName -ErrorAction SilentlyContinue }

    Write-Ok "installed $Tag"

    # ------------------------------------------------- external tools -------
    # HooCode fetches these on first use anyway; pre-seeding just means the first
    # run is immediate. The asset names mirror src/utils/tools-manager.ts - if
    # that table changes, this one has to change with it. A mismatch 404s and the
    # install carries on without the tool, which is the same state as skipping it.
    function Get-ToolRepo {
        param([string] $Tool)
        switch ($Tool) {
            'fd'         { 'sharkdp/fd' }
            'rg'         { 'BurntSushi/ripgrep' }
            'embsearch'  { 'kolisachint/embeddingsearchtools' }
            'webtools'   { 'kolisachint/webtools' }
            'voicetools' { 'kolisachint/voicetools' }
            default      { $null }
        }
    }

    function Get-ToolAsset {
        param([string] $Tool, [string] $ToolVersion)
        $a = if ($Arch -eq 'arm64') { 'aarch64' } else { 'x86_64' }
        switch ($Tool) {
            'fd'         { "fd-v$ToolVersion-$a-pc-windows-msvc.zip" }
            'rg'         { "ripgrep-$ToolVersion-$a-pc-windows-msvc.zip" }
            'embsearch'  { "embsearch-$a-pc-windows-msvc.zip" }
            'webtools'   { "webtools-$a-pc-windows-msvc.zip" }
            'voicetools' { "voicetools-$a-pc-windows-msvc.zip" }
            default      { $null }
        }
    }

    function Install-Tool {
        param([string] $Tool)

        $toolRepo = Get-ToolRepo $Tool
        if (-not $toolRepo) { Write-Warn "unknown tool: $Tool"; return }

        try {
            $rel = Invoke-RestMethod -Uri "https://api.github.com/repos/$toolRepo/releases/latest" -Headers @{ 'User-Agent' = 'hoocode-installer' }
            $toolTag = $rel.tag_name
        } catch {
            Write-Warn "${Tool}: could not resolve a release; HooCode will fetch it on first use."
            return
        }

        $assetName = Get-ToolAsset $Tool ($toolTag -replace '^v', '')
        $dl = Join-Path $TmpDir $assetName
        try {
            Invoke-WebRequest -Uri "https://github.com/$toolRepo/releases/download/$toolTag/$assetName" -OutFile $dl -UseBasicParsing
        } catch {
            Write-Warn "${Tool}: no $Arch Windows build published; HooCode falls back to its built-in path."
            return
        }

        $ex = Join-Path $TmpDir "x-$Tool"
        try {
            Expand-Archive -Path $dl -DestinationPath $ex -Force
        } catch {
            Write-Warn "${Tool}: archive did not unpack."
            return
        }

        # Upstream archives vary between a flat layout and one directory deep, so
        # search rather than assume.
        $found = Get-ChildItem -Path $ex -Recurse -File -Filter "$Tool.exe" -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $found) {
            Write-Warn "${Tool}: no $Tool.exe inside the archive."
            return
        }

        Copy-Item -Path $found.FullName -Destination (Join-Path $BinDir "$Tool.exe") -Force
        Unblock-File -Path (Join-Path $BinDir "$Tool.exe") -ErrorAction SilentlyContinue
        Write-Ok "+ $Tool $toolTag"
    }

    if ($NoTools) {
        Write-Step 'Skipping external tools (-NoTools). HooCode fetches what it needs on demand.'
    } else {
        Write-Step "Pre-seeding external tools into $BinDir..."
        foreach ($t in $Tools) { Install-Tool $t }
    }

    # ------------------------------------------------------------- PATH -----
    $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $UserPath) { $UserPath = '' }
    # The @() is load-bearing. Where-Object hands back $null when nothing
    # matched and a bare string when one entry did, and under Set-StrictMode
    # neither of those has a .Count - so the unwrapped form threw on every run,
    # first install and re-install alike, right before the PATH was written.
    $OnPath = @($UserPath -split ';' | Where-Object { $_ -and ($_.TrimEnd('\') -ieq $BinDir.TrimEnd('\')) }).Count -gt 0

    if ($OnPath) {
        # Already there from a previous install.
    } elseif ($NoModifyPath) {
        Write-Warn "$BinDir is not on your PATH. Add it yourself, or re-run without -NoModifyPath."
    } else {
        $newPath = if ($UserPath.TrimEnd(';')) { "$($UserPath.TrimEnd(';'));$BinDir" } else { $BinDir }
        [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
        # Also for this session, so the user can run hoocode without reopening.
        $env:Path = "$env:Path;$BinDir"
        Write-Step "Added $BinDir to your user PATH"
        Write-Note 'open a new terminal for other shells to pick it up'
    }

    # ------------------------------------------------------------- done -----
    Write-Host ''
    Write-Host 'HooCode is installed.' -ForegroundColor Green
    Write-Host ''
    Write-Host '  hoocode          ' -NoNewline -ForegroundColor White; Write-Host 'start in build mode (or `hoo`, same thing)'
    Write-Host '  hoocode --help   ' -NoNewline -ForegroundColor White; Write-Host 'every flag'
    Write-Host '  /login           ' -NoNewline -ForegroundColor White; Write-Host 'pick a provider once you are in'
    Write-Host ''
    Write-Host "  Docs      $Website" -ForegroundColor Cyan
    Write-Host "  Source    https://github.com/$Repo" -ForegroundColor Cyan
    Write-Host ''
    Write-Host '  * ' -NoNewline -ForegroundColor Yellow
    Write-Host 'If HooCode saves you time, a star is the cheapest way to say so:'
    Write-Host "    https://github.com/$Repo" -ForegroundColor Cyan
    Write-Host ''
} finally {
    $ProgressPreference = $PrevProgress
    Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue
}
