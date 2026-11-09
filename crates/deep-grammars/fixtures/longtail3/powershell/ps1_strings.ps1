# Every string form: single quotes, expanding strings, and here-strings.
$name = 'a literal $name and `tick`'
$greeting = "hello $name, you are in $env:USERNAME"
$count = "total: $(($items | Measure-Object).Count)"
$nested = "path is $($root.TrimEnd('\')) and $($parts[0])"

$report = @"
Build: $name
Count: $($items.Count)
Path:  $($root)
"@

$literal = @'
No $expansion here at all.
'@

$escaped = "a `"quoted`" word and a `$literal dollar"
$multi = "line one
line two and $name"

Write-Output $greeting
Write-Output $count
Write-Output $report
Write-Output "$literal$escaped"
