cat /v/tsp.log /v/imports.txt /bs.commit; export NODE_PATH=/bs/src/node_modules
find /bs/examples -name "*.ifcx" | sort | head -60 | while IFS= read -r f; do
  echo "CONTROL $f"; rm -f /tmp/c.json
  timeout 300 node /v/ifcx-cli.cjs compose --no-fetch /imports/*.ifcx "$f" /tmp/c.json 2>&1 | grep -m1 -iE 'error|invalid|missing|expected' | cut -c1-250 | tr '\n' ' '; echo "compose-rc=${PIPESTATUS[0]}"
  node /v/ifcxschema.mjs "$f" 2>&1 | tail -1
done
