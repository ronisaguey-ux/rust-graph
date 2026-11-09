#!/bin/bash
# Every here-document form, one body per form.
set -euo pipefail

# A plain body expands.
cat <<EOF
plain $HOME $(hostname)
EOF

# The delimiter may be indented with tabs and the body keeps its own tabs.
if true; then
	cat <<-EOF
		stripped body
		$((1 + 2))
	EOF
fi

# A quoted delimiter and an escaped one do not expand.
cat <<'EOF'
literal $HOME `date` $(date)
EOF

cat <<"EOF"
also $literal
EOF

cat <<\EOF
backslash $literal
EOF

# Several here-documents on one line, bodies in order.
cat <<A <<B
first $(one)
A
second `two`
B

# Inside a function, a loop and a pipeline.
function emit {
	cat <<EOF
from emit: ${1:-none}
EOF
}

for target in a b; do
	cat <<EOF | grep "$target"
$(build "$target")
EOF
done

# Nested command substitutions and backticks inside a body.
cat <<EOF
outer $(inner $(deep))
tick `date +%s`
param ${value:-$(fallback)}
EOF
