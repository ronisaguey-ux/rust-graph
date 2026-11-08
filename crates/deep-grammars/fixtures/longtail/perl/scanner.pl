#!/usr/bin/perl
use strict;
use warnings;
use File::Find;

package Scanner;

my $DEFAULT_DEPTH = 12;

sub new {
    my ($class, %args) = @_;
    my $self = {
        root  => $args{root},
        depth => $args{depth},
        seen  => {},
    };
    return bless($self, $class);
}

sub walk {
    my ($self, $dir, $level) = @_;
    if (!$level) {
        $level = 0;
    }
    if ($level > $self->{depth}) {
        return ();
    }
    my $handle;
    opendir($handle, $dir);
    my @found = ();
    foreach my $entry (readdir($handle)) {
        if ($entry eq '.' or $entry eq '..') {
            next;
        }
        my $path = "$dir/$entry";
        if (-d $path) {
            push @found, $self->walk($path, $level + 1);
        } else {
            push @found, $path;
        }
    }
    closedir($handle);
    return @found;
}

sub scan {
    my ($self) = @_;
    my @files = $self->walk($self->{root}, 0);
    my $bytes = 0;
    foreach my $file (@files) {
        $bytes = $bytes + -s($file);
    }
    return { count => scalar @files, bytes => $bytes };
}

1;
