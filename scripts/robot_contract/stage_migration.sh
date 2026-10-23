#!/usr/bin/env bash
# Stage metadata-only schemaVersion 2 overlays of machine packages.
#
#   stage_migration.sh OUT GEARBOX ASSET_ROOT [NAME:ROOT:BODY...]
#
# OUT/packages/<name>.usdz is a copy of the original, OUT/v2/<name>_v2.usda
# sublayers it by relative path and adds the version and a declared control
# frame on BODY (the legacy forward -Y turned into +X), plus the reviewed
# per-asset metadata in migrations/<name>.overs when present; the command
# interfaces v1 derived are authored in OUT/layers/<name>_interfaces.usda. OUT/manifests holds
# the compiled manifest of both, OUT/reports the before/after comparison.
# Originals are only read.
set -euo pipefail

out=$1 gearbox=$2 assets=$3
shift 3
here=$(cd "$(dirname "$0")" && pwd)
mkdir -p "$out/packages" "$out/layers" "$out/v2" "$out/manifests" "$out/reports"

# Without NAME:ROOT:BODY arguments, every package under ASSET_ROOT.
specs=("$@")
if [[ ${#specs[@]} -eq 0 ]]; then
    for package in "$assets"/*.usdz; do
        name=$(basename "$package" .usdz)
        specs+=("$name:$("$gearbox" describe-machines "$package" 2>/dev/null | python3 -I -c '
import json, sys
m = json.load(sys.stdin)["machines"][0]
print(m["root"] + ":" + m["control_body"])
')")
    done
fi

for spec in "${specs[@]}"; do
    IFS=: read -r name root body <<<"$spec"
    cp "$assets/$name.usdz" "$out/packages/$name.usdz"
    "$gearbox" describe-machines "$out/packages/$name.usdz" >"$out/manifests/$name.v1.json" 2>"$out/manifests/$name.v1.log"

    read -r mpu kpu up < <(python3 -I -c '
import json, sys
m = json.load(open(sys.argv[1]))["machines"][0]["units"]
print(m["meters_per_unit"]["value"], m["kilograms_per_unit"]["value"], m["up_axis"]["value"].upper())
' "$out/manifests/$name.v1.json")

    leaf=${body#"$root"/}
    overlay="$out/v2/${name}_v2.usda"
    python3 -I "$here/interface_overs.py" "$out/manifests/$name.v1.json" "$out/layers/${name}_interfaces.usda"
    {
        printf '#usda 1.0\n(\n'
        printf '    defaultPrim = "%s"\n' "${root#/}"
        printf '    metersPerUnit = %s\n    kilogramsPerUnit = %s\n    upAxis = "%s"\n' "$mpu" "$kpu" "$up"
        printf '    subLayers = [@../layers/%s_interfaces.usda@, @../packages/%s.usdz@]\n)\n\n' "$name" "$name"
        printf 'over "%s"\n{\n' "${root#/}"
        printf '    int gearbox:machine:schemaVersion = 2\n'
        printf '    rel gearbox:machine:controlFrame = <%s/control_frame>\n' "$body"
        depth=1
        IFS=/ read -ra parts <<<"$leaf"
        for part in "${parts[@]}"; do
            printf '%*sover "%s"\n%*s{\n' $((depth * 4)) '' "$part" $((depth * 4)) ''
            depth=$((depth + 1))
        done
        pad=$((depth * 4))
        printf '%*sdef Xform "control_frame"\n%*s{\n' $pad '' $pad ''
        printf '%*squatd xformOp:orient = (0.7071067811865476, 0, 0, -0.7071067811865476)\n' $((pad + 4)) ''
        printf '%*suniform token[] xformOpOrder = ["xformOp:orient"]\n' $((pad + 4)) ''
        printf '%*s}\n' $pad ''
        for ((i = depth - 1; i >= 1; i--)); do
            printf '%*s}\n' $((i * 4)) ''
        done
        if [[ -f "$here/migrations/$name.overs" ]]; then
            cat "$here/migrations/$name.overs"
        fi
        printf '}\n'
    } >"$overlay"

    "$gearbox" describe-machines "$overlay" >"$out/manifests/$name.v2.json" 2>"$out/manifests/$name.v2.log" || true
    python3 -I "$here/migration_report.py" "$out/manifests/$name.v1.json" "$out/manifests/$name.v2.json" "$out/reports/$name.json"
done
