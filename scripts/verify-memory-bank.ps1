[CmdletBinding()]
param(
    [string]$Root = (Split-Path -Parent $PSScriptRoot)
)

$ErrorActionPreference = 'Stop'
$requiredFiles = @(
    'AGENTS.md',
    'memory-bank/README.md',
    'memory-bank/projectbrief.md',
    'memory-bank/productContext.md',
    'memory-bank/systemPatterns.md',
    'memory-bank/techContext.md',
    'memory-bank/activeContext.md',
    'memory-bank/progress.md',
    'memory-bank/decisions.md',
    'memory-bank/certification-matrix.md',
    'memory-bank/tasks/_index.md',
    'memory-bank/tasks/ROADMAP.md',
    'memory-bank/tasks/TASK001-foundation.md',
    'memory-bank/tasks/TASK002-device-discovery-identity.md',
    'memory-bank/tasks/TASK003-ingest-copy-sort.md',
    'memory-bank/tasks/TASK004-verification-recovery.md',
    'memory-bank/tasks/TASK005-safe-format.md',
    'memory-bank/tasks/TASK006-sandisk-slot-mapping.md',
    'memory-bank/tasks/TASK007-local-store-profiles.md',
    'memory-bank/tasks/TASK008-operator-ui.md',
    'memory-bank/tasks/TASK009-security-destructive-safety.md',
    'memory-bank/tasks/TASK010-cross-platform-hardware-certification.md',
    'memory-bank/tasks/TASK011-packaging-release-support.md',
    'memory-bank/tasks/TASK012-ingest-lifecycle-receipts.md'
)

$missing = $requiredFiles | Where-Object { -not (Test-Path -LiteralPath (Join-Path $Root $_) -PathType Leaf) }
if ($missing) {
    throw "Missing required AI-context file(s): $($missing -join ', ')"
}

$markdownFiles = Get-ChildItem -LiteralPath (Join-Path $Root 'memory-bank') -Filter '*.md' -Recurse -File
$brokenLinks = [System.Collections.Generic.List[string]]::new()
foreach ($file in $markdownFiles) {
    $matches = [regex]::Matches((Get-Content -LiteralPath $file.FullName -Raw), '\[[^\]]+\]\(([^)#]+)(?:#[^)]*)?\)')
    foreach ($match in $matches) {
        $target = $match.Groups[1].Value
        if ($target -match '^[a-z]+:' -or $target.StartsWith('/')) { continue }
        if (-not (Test-Path -LiteralPath (Join-Path $file.DirectoryName $target))) {
            $brokenLinks.Add("$($file.FullName): $target")
        }
    }
}

if ($brokenLinks.Count) {
    throw "Broken local Markdown link(s):`n$($brokenLinks -join "`n")"
}

$taskFiles = Get-ChildItem -LiteralPath (Join-Path $Root 'memory-bank/tasks') -Filter 'TASK*.md' -File
if ($taskFiles.Count -ne 12) {
    throw "Expected 12 implementation task files, found $($taskFiles.Count)."
}

$taskProblems = [System.Collections.Generic.List[string]]::new()
foreach ($taskFile in $taskFiles) {
    $taskContent = Get-Content -LiteralPath $taskFile.FullName -Raw
    foreach ($section in @('Status', 'Depends', 'Acceptance', 'Evidence')) {
        if ($taskContent -notmatch $section) {
            $taskProblems.Add("$($taskFile.Name): missing $section")
        }
    }
}

if ($taskProblems.Count) {
    throw "Incomplete task file(s):`n$($taskProblems -join "`n")"
}

Write-Host "Memory bank validation passed ($($markdownFiles.Count) Markdown files and $($taskFiles.Count) task files checked)."
