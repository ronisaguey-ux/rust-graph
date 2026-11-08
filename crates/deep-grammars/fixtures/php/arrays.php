<?php
$items = [1, 2, 3];
$items[] = 4;
$matrix = [[1, 2], [3, 4]];
$total = 0;
foreach ($items as $index => $value) {
    $total = $total + $value * $index;
}
$keyed = ["a" => 1, "b" => 2];
$copy = [...$items, 5];
$first = $items[0];
unset($items[0]);
$exists = isset($keyed["a"]);
$missing = $keyed["z"] ?? 0;
echo $total;
echo $matrix[1][0];
echo $copy[0];
echo $first;
echo $exists;
echo $missing;
