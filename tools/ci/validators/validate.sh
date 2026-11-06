#!/bin/bash
# validate.sh <dir with exports named x.ifc x.ifcx x.frag x.usda x.usdc x.glb> -> JSON lines "fmt {...}"
d=$1; cd /v
j(){ echo "$1 $(cat)"; echo "time $1 $(( $(date +%s) - T0 ))"; T0=$(date +%s); }; T0=$(date +%s)
/venv/bin/python ifcv.py $d/x.ifc 2>&1 | tail -1 | j ifc
node gv.mjs $d/x.glb 2>&1 | tail -1 | j gltf
/venv/bin/python usdv.py $d/x.usda 2>&1 | tail -1 | j usda
/venv/bin/python usdv.py $d/x.usdc 2>&1 | tail -1 | j usdc
node fragcheck.mjs $d/x.frag 2>&1 | tail -1 | j frag
( rm -f $d/composed.json; export NODE_PATH=/bs/src/node_modules
  timeout 600 node /v/ifcx-cli.cjs compose --no-fetch /imports/*.ifcx $d/x.ifcx $d/composed.json > $d/ifcx.log 2>&1; rc=$?
  how=cli
  if [ $rc != 0 ] && grep -q "Invalid string length" $d/ifcx.log && [ -f /v/ifcx-load.cjs ]; then
    timeout 900 node --max-old-space-size=12000 /v/ifcx-load.cjs /imports/*.ifcx $d/x.ifcx > $d/ifcx.log 2>&1; rc=$?; how=cli-without-output-file
  fi
  echo "rc=$rc how=$how bytes=$(stat -c%s $d/composed.json 2>/dev/null || echo 0) $(grep -m1 -iE 'error|invalid|missing|expected' $d/ifcx.log | cut -c1-300 | tr -d '"')" ) | j ifcx
node /v/ifcxschema.mjs $d/x.ifcx 2>&1 | tail -1 | j ifcxschema
