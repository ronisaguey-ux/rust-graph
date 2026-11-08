param(
    [string]$Configuration = "Debug",
    [int]$Retries = 3
)

function Get-BuildPath {
    param([string]$Name)
    return Join-Path -Path $Configuration -ChildPath $Name
}

$items = Get-ChildItem -Path "." -Filter "*.cs"
foreach ($item in $items) {
    Write-Host $item.Name
}

$count = 0
if ($count -gt 0) {
    $count = $count + 1
} elseif ($count -eq 0) {
    Write-Host "empty"
} else {
    Write-Host "negative"
}

for ($index = 0; $index -lt 3; $index++) {
    Write-Host $index
}
