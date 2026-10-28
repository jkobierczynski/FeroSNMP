#!/usr/bin/env bash
# Download the corpus MIBs that cannot be committed to this repository and
# verify every file in the corpus against manifest.tsv.
#
# Usage: corpus/fetch.sh            download missing files, verify all
#        corpus/fetch.sh --verify   verify only, download nothing
#
# Needs bash, curl and sha256sum (all present in Git Bash on Windows).
set -euo pipefail

cd "$(dirname "$0")"
verify_only=0
[[ "${1:-}" == "--verify" ]] && verify_only=1

fetched=0 failed=0 total=0
while IFS=$'\t' read -r path modules group redistributable licence origin url sha256; do
    [[ "$path" == "path" ]] && continue   # header row
    total=$((total + 1))

    if [[ ! -f "$path" ]]; then
        if [[ "$redistributable" == "yes" ]]; then
            echo "MISSING  $path (committed file, should be in the checkout)" >&2
            failed=$((failed + 1))
            continue
        fi
        if [[ $verify_only -eq 1 ]]; then
            echo "MISSING  $path (run corpus/fetch.sh to download)" >&2
            failed=$((failed + 1))
            continue
        fi
        mkdir -p "$(dirname "$path")"
        if ! curl -fsSL --retry 3 -o "$path.part" "$url"; then
            echo "FAILED   $path <- $url" >&2
            rm -f "$path.part"
            failed=$((failed + 1))
            continue
        fi
        mv "$path.part" "$path"
        fetched=$((fetched + 1))
    fi

    actual=$(sha256sum "$path" | cut -d' ' -f1)
    if [[ "$actual" != "$sha256" ]]; then
        echo "CHECKSUM $path: expected $sha256, got $actual" >&2
        failed=$((failed + 1))
    fi
done < manifest.tsv

echo "corpus: $total files, $fetched downloaded, $failed problems"
[[ $failed -eq 0 ]]
