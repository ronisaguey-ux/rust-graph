<?php
$score = 72;

if ($score >= 90) {
    echo "A";
} elseif ($score >= 80) {
    echo "B";
} elseif ($score >= 70) {
    echo "C";
} else {
    echo "F";
}

$count = 0;
while ($count < 3) {
    $count++;
}

do {
    $count--;
} while ($count > 0);

for ($i = 0; $i < 5; $i++) {
    if ($i % 2 == 0) {
        continue;
    }
    echo $i;
}

switch ($score) {
    case 90:
        echo "top";
        break;
    case 72:
        echo "mid";
        break;
    default:
        echo "other";
}

while (true) {
    break;
}
