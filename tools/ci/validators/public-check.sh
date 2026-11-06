#!/bin/bash
# Public CI format check: export public samples to IFC, IFCX, .frag, USDA, USDC and glTF and run each format's
# official validator (the image in this folder). Usage: public-check.sh BIN_DIR SAMPLE_FILE...
set -u; V=$(cd "$(dirname "$0")" && pwd); BIN=$1; shift; W=${RUNNER_TEMP:-/tmp}/fmt; rm -rf "$W"; mkdir -p "$W"
docker build -q -t val "$V" > "$W/build.log" 2>&1 || { echo "FORMATS-CHECK image build failed"; tail -5 "$W/build.log"; exit 1; }
for p in "$@"; do d=$W/$(basename "$p" .rvt | tr ' ' _); mkdir -p "$d"
  timeout 900 "$BIN/rvt-ifc" "$p" -o "$d/x.ifc" --ifcx "$d/x.ifcx" --frag "$d/x.frag" --usda "$d/x.usda" --usdc "$d/x.usdc" > "$d/e1.log" 2>&1; r1=$?
  timeout 900 "$BIN/rvt-gltf" "$p" -o "$d/x.glb" > "$d/e2.log" 2>&1; r2=$?
  [ $r1 = 0 ] && docker run --rm -v "$d":/d val bash /v/validate.sh /d > "$d/val.txt" 2>&1 || : > "$d/val.txt"
  python3 - "$(basename "$p")" $r1 $r2 "$d" >> "$W/results.jsonl" <<'P'
import sys,json
p,r1,r2,d=sys.argv[1:]; r={"file":p,"rc_ifc":int(r1),"rc_gltf":int(r2)}
for l in open(d+"/val.txt"):
    k,_,v=l.partition(" ")
    if k in("ifc","gltf","usda","usdc","frag","ifcx","ifcxschema"):
        try: r[k]=json.loads(v)
        except Exception: r[k]={"unparsed":l[:200]}
print(json.dumps(r))
P
done
python3 "$V/summarize.py" "$W/results.jsonl"
