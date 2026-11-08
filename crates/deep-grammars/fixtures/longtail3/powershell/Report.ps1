function Show-Report {
    param([string]$Title)

    switch ($Title) {
        "daily" { Write-Host "daily report" }
        "weekly" { Write-Host "weekly report" }
        default { Write-Host "other" }
    }
}

$total = 0
do {
    $total = $total + 1
} while ($total -lt 5)

while ($total -gt 0) {
    $total -= 1
}

Show-Report -Title "daily"
Write-Output $total
