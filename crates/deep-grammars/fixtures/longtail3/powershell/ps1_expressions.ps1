param(
    [Parameter(Mandatory = $false)]
    [string]$Root = ".",
    [int]$Depth = 3
)

$files = Get-ChildItem -Path $Root -Recurse -Depth $Depth -File |
    Where-Object -FilterScript { $_.Length -gt 0 } |
    Sort-Object -Property Length -Descending

$total = ($files | Measure-Object -Property Length -Sum).Sum
Write-Verbose "found $($files.Count) files, $total bytes"

$roots = @("src", "tests", "docs")
$lookup = @{
    Name    = "stack"
    Version = "1.2.3"
    Roots   = $roots
}

foreach ($item in $lookup.Roots) {
    if (-not (Test-Path -Path $item)) {
        Write-Warning "missing $item"
        continue
    }
    $count = @(Get-ChildItem -Path $item -Filter *.ps1).Count
    Write-Host "$item has $count scripts"
}

Get-ChildItem -Path $Root | ForEach-Object { $_.Name }
Get-Item @args
$first = $PSItem
$value = $env:COMPUTERNAME
$sum = (1..10 | Measure-Object -Sum).Sum
