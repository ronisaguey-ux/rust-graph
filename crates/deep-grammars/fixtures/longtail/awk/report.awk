# Emit a CSV summary of files larger than 1 MB.
BEGIN {
    FS = " "
    count = 0
}

$5 > 1048576 {
    count = count + 1
    printf "%s,%d\n", $9, $5
}

END {
    if (count == 0) {
        print "no large files"
    } else {
        printf "total,%d\n", count
    }
}
