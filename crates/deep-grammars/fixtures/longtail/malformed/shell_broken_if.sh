#!/bin/sh
# A broken if: `then` never arrives and the quote is left open.
echo "starting"
if [ -f /etc/hosts ]
    echo "hosts exists
fi
