#!/bin/bash
# case/esac, [[ ]], arrays, $(( )), process substitution and ${...} forms.

declare -a targets=("one" "two" "$(three)")
declare -A seen=([one]=1 [two]=2)
count=$((10 + 2 * (3 + 4)))
((count++))

case "${1:-default}" in
	(a | b) echo "ab" ;;
	c) echo "c" ;;
	*.tar.gz) echo "archive" ;;
	*) echo "other" ;;
esac

if [[ -f "$config" && -n "${HOME}" ]] && [[ $mode =~ ^(prod|stage)$ ]]; then
	echo "configured"
elif [[ $mode == "dev" ]]; then
	echo "dev"
fi

while read -r line; do
	echo "${line%%:*}"
done < <(grep -v '^#' /etc/hosts)

diff <(sort left.txt) <(sort right.txt) > /dev/null || true

for ((i = 0; i < ${#targets[@]}; i++)); do
	echo "${targets[$i]}: ${targets[$i]:-missing}"
done

path=${PATH:+$PATH:}/usr/local/bin
name=${1:?usage: script <name>}
trimmed=${raw#prefix}
echo "$((count * 2)) ${path} ${name} ${trimmed}"
