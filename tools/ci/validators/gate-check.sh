#!/bin/bash
# Gate stage: export sample models to every format and run each format's official validator on them.
# Runs on the gate VM after the build: binaries in /var/gate/target/ci, samples under /rvt. Needs docker and network (image build).
set -u; V=$(cd "$(dirname "$0")" && pwd); W=/var/gate/fmt; BIN=/var/gate/target/ci; rm -rf $W; mkdir -p $W
docker build -q -t val "$V" > $W/build.log 2>&1 || { echo "FORMATS-CHECK image build failed"; tail -5 $W/build.log; exit 1; }
cat > $W/list.txt <<L
autodesk-rvt-samples-2024/racbasicsampleproject.rvt
autodesk-rvt-samples-2024/racadvancedsampleproject.rvt
autodesk-rvt-samples-2024/rstbasicsampleproject.rvt
autodesk-rvt-samples-2024/rmebasicsampleproject.rvt
autodesk-rvt-samples-2014/Technical_school-current_m.rvt
nordic-lca-bim4lca/ARCH/REVIT/ARK_NordicLCA_Housing_Concrete_BuildingPermit.rvt
L
one() { p=$1; d=$W/$(echo "$p" | tr '/ ' '__'); mkdir -p $d
  timeout 900 $BIN/rvt-ifc /rvt/$p -o $d/x.ifc --ifcx $d/x.ifcx --frag $d/x.frag --usda $d/x.usda --usdc $d/x.usdc > $d/e1.log 2>&1; r1=$?
  timeout 900 $BIN/rvt-gltf /rvt/$p -o $d/x.glb > $d/e2.log 2>&1; r2=$?
  [ $r1 = 0 ] && docker run --rm -v $d:/d val bash /v/validate.sh /d > $d/val.txt 2>&1 || : > $d/val.txt
  python3 - "$p" $r1 $r2 $d >> $W/results.jsonl <<'P'
import sys,json
p,r1,r2,d=sys.argv[1:]; r={"file":p,"rc_ifc":int(r1),"rc_gltf":int(r2)}
for l in open(d+"/val.txt"):
    k,_,v=l.partition(" ")
    if k in("ifc","gltf","usda","usdc","frag","ifcx","ifcxschema"):
        try: r[k]=json.loads(v)
        except Exception: r[k]={"unparsed":l[:200]}
print(json.dumps(r))
P
  rm -rf $d; }
export -f one; export W BIN
cat $W/list.txt | xargs -P 3 -I{} bash -c 'one {}'
python3 $V/summarize.py $W/results.jsonl
