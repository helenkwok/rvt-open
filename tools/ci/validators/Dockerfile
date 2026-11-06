FROM node:22
RUN apt-get update -q && apt-get install -yq python3-venv git && python3 -m venv /venv && /venv/bin/pip install -q --no-cache-dir --default-timeout=180 --retries 10 ifcopenshell==0.9.0.post1 usd-core==26.11
WORKDIR /v
RUN npm i --ignore-scripts --save-exact gltf-validator@2.0.0-dev.3.10 @thatopen/fragments@3.6.0 three@0.185.1 web-ifc@0.0.77
RUN git clone -q https://github.com/buildingSMART/IFC5-development /bs && git -C /bs rev-parse HEAD > /bs.commit && cd /bs/src && npm ci --ignore-scripts > /tmp/ifcxnpm.log 2>&1; npx esbuild ifcx-cli/ifcx-cli.ts --bundle --outfile=/v/ifcx-cli.cjs --external:three --platform=node > /tmp/esb.log 2>&1; ls -la /v/ifcx-cli.cjs
RUN cd /v && npm i --ignore-scripts --save-exact ajv@8.17.1 ajv-formats@3.0.1 yaml@2.6.1 >/dev/null 2>&1; cd /bs/schema && (npm ci --ignore-scripts > /tmp/tsp.log 2>&1; npx tsp compile ifcx.tsp >> /tmp/tsp.log 2>&1; echo tsp-rc=$? >> /tmp/tsp.log; find . -path ./node_modules -prune -o -type f \( -name '*.yaml' -o -name '*.json' \) -newer package.json -print >> /tmp/tsp.log); git -C /bs log -1 --format='%H %cs' >> /bs.commit; cp /tmp/tsp.log /v/tsp.log
RUN mkdir /imports && cd /imports && for u in @standards.buildingsmart.org/ifc/core/ifc@v5a.ifcx @standards.buildingsmart.org/ifc/core/prop@v5a.ifcx @standards.buildingsmart.org/ifc/ifc-mat/ifc-mat@v1.0.0.ifcx @openusd.org/usd@v1.ifcx @nlsfb/nlsfb@v1.ifcx; do curl -sfL https://ifcx.dev/$u -o /tmp/imp.json && python3 -c "import json,sys;d=json.load(open('/tmp/imp.json'));d['header']['id']=sys.argv[1];json.dump(d,open(sys.argv[2],'w'))" https://ifcx.dev/$u /imports/$(basename $u); done; ls -l /imports > /v/imports.txt
COPY ifcx-load.ts /bs/src/ifcx-cli/ifcx-load.ts
RUN cd /bs/src && npx esbuild ifcx-cli/ifcx-load.ts --bundle --outfile=/v/ifcx-load.cjs --external:three --platform=node > /tmp/esb2.log 2>&1; ls /v/ifcx-load.cjs || true
COPY fragcheck.mjs ifcxschema.mjs control.sh gv.mjs ifcv.py usdv.py validate.sh /v/
