#!/usr/bin/perl
use strict;
use warnings;

my %summary = (count => 0, bytes => 0, elapsed => 0.0);

sub row {
    my ($name, $value) = @_;
    return sprintf("| %-16s | %10d |", $name, $value);
}

sub render {
    my ($summary) = @_;
    my @lines = ("## Scanner summary", "");
    push @lines, row('files', $summary->{count});
    push @lines, row('bytes', $summary->{bytes});
    return join("\n", @lines) . "\n";
}

$summary{count} = 1843;
$summary{bytes} = 9_123_456;
print render(\%summary);
