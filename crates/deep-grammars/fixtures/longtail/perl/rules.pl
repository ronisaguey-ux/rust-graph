package Rules;

use strict;

our @RULES = (
    { name => 'size',  max => 1_000_000 },
    { name => 'depth', max => 8 },
);

sub size_ok {
    my ($path) = @_;
    my $size = -s $path;
    return defined $size && $size <= 1_000_000;
}

sub depth_ok {
    my ($path) = @_;
    my @parts = split("/", $path);
    return scalar @parts <= 8;
}

sub evaluate {
    my ($class, @files) = @_;
    my @kept;
    foreach my $file (@files) {
        if (size_ok($file) && depth_ok($file)) {
            push @kept, $file;
        }
    }
    return @kept;
}

1;
