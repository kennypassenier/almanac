#!/usr/bin/env bash
# Converts v1 mapping profiles to the 2.0.0 routing shape.
#
# 2.0.0 refuses a v1 profile rather than misreading it, so almanac will
# not start until every profile has been converted. That refusal is
# right — silently ignoring a [mapping] block would drop everything the
# profile says — but it means the conversion and the upgrade have to
# happen in the same window.
#
# Run it on the machine, between stopping the old version and starting
# the new one:
#
#   systemctl stop almanac
#   ./migrate-profiles-v2.sh /appdata/almanac/almanac-config/profiles
#   <install 2.0.0>
#   systemctl start almanac
#
# Keeps every original as <name>.toml.v1 so going back is a rename.
#
# What it CANNOT do, and says so per file: a v1 profile that mapped
# unusual field names (title_field = "monitor.name") described a source
# that speaks a different shape. 2.0.0 has no translation layer — that
# source needs HTTPSwitchboard in front of it, or needs to send
# almanac's shape. The conversion still runs; the source will get a 422
# naming the field it did not send, which is the honest outcome.
set -euo pipefail

dir=${1:?usage: migrate-profiles-v2.sh <profiles directory>}
[ -d "$dir" ] || { echo "no such directory: $dir" >&2; exit 1; }

shopt -s nullglob
converted=0
for file in "$dir"/*.toml; do
    if ! grep -q '^schema_version[[:space:]]*=[[:space:]]*1[[:space:]]*$' "$file"; then
        echo "skipping $(basename "$file"): not a v1 profile"
        continue
    fi

    value() { sed -n "s/^$1[[:space:]]*=[[:space:]]*\"\(.*\)\"[[:space:]]*$/\1/p" "$file" | head -1; }
    source_id=$(value source_id)
    calendar=$(value target_calendar_id)
    timezone=$(value timezone)
    : "${timezone:=Europe/Brussels}"

    if [ -z "$source_id" ] || [ -z "$calendar" ]; then
        echo "REFUSING $(basename "$file"): no source_id or target_calendar_id found" >&2
        exit 1
    fi

    # Warn where the old profile was translating rather than routing:
    # a field name that is not one almanac now defines describes a
    # source speaking someone else's shape.
    odd=$(grep -oE '^[a-z_]+_field[[:space:]]*=[[:space:]]*"[^"]+"' "$file" \
        | grep -vE '"(title|description|start|location|end)"' || true)
    if [ -n "$odd" ]; then
        echo "NOTE $(basename "$file"): this profile translated field names:"
        echo "$odd" | sed 's/^/      /'
        echo "      2.0.0 has no translation layer. Either this source sends almanac's"
        echo "      shape, or it needs HTTPSwitchboard in front of it."
    fi

    cp -a "$file" "$file.v1"
    cat > "$file" <<PROFILE
# Converted from the v1 mapping format on $(date -Iseconds).
# The original is beside this file as $(basename "$file").v1.
#
# A profile now says only where this source's events land; what each
# event IS comes from the call. See docs/USER_GUIDE.md.

schema_version = 2
source_id = "$source_id"
target_calendar_id = "$calendar"
timezone = "$timezone"
default_duration_minutes = 60
PROFILE
    echo "converted $(basename "$file")"
    converted=$((converted + 1))
done

echo "$converted profile(s) converted; originals kept as *.toml.v1"
