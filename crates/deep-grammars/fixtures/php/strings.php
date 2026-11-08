<?php
$name = "world";
$greeting = "hello {$name} and $name";
$single = 'no $interpolation here';
$dollar = "price: \$5";
$combined = $greeting . " " . $name;

$sql = <<<SQL
SELECT * FROM users
WHERE name = 'ada'
SQL;

$doc = <<<'TEXT'
A nowdoc keeps $name literal.
TEXT;

$words = ["alpha", "beta", "gamma"];
$map = array("host" => "localhost", "port" => 8080);
$nested = $map["host"];
echo $combined;
echo $sql;
echo $doc;
echo $words[1];
echo $nested;
