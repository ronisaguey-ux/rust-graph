<?php
require_once __DIR__ . "/bootstrap.php";
require "helpers.php";

function sum(int ...$values): int
{
    $total = 0;
    foreach ($values as $value) {
        $total += $value;
    }
    return $total;
}

function greet(string $name = "world", bool $loud = false)
{
    $message = "Hello, {$name}!";
    if ($loud) {
        return strtoupper($message);
    }
    return $message;
}

$result = sum(1, 2, 3);
echo greet("ada", true), "\n";
echo $result;
