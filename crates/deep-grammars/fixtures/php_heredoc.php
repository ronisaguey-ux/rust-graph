<?php
$sql = <<<SQL
SELECT * FROM t WHERE id = {$id}
SQL;
$doc = <<<EOT
hello {$user->name()}
EOT;
$nowdoc = <<<'TXT'
literal $notInterpolated and {$not_a_field}
TXT;
$indented = <<<IND
  body {$value}
  IND;
