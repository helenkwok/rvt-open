// validate an .ifcx against the OpenAPI IfcxFile schema regenerated from schema/ifcx.tsp. The `path` scalar's pattern
// (angle-bracketed) is counted separately and then dropped: every buildingSMART example violates it, and their compose accepts them.
import fs from "fs"; import YAML from "yaml"; import Ajv from "ajv/dist/2020.js"; import addFormats from "ajv-formats";
const oa = YAML.parse(fs.readFileSync(process.env.OA || "/bs/schema/out/@typespec/openapi3/openapi.yaml","utf8"));
const pat = new RegExp(oa.components.schemas.path?.pattern ?? ".*"); delete oa.components.schemas.path?.pattern;
const root = { $id:"oa", ...oa, $ref:"#/components/schemas/IfcxFile" };
const ajv = new Ajv({strict:false, allErrors:true}); addFormats(ajv); const v = ajv.compile(root);
for (const p of process.argv.slice(2)) { const d = JSON.parse(fs.readFileSync(p,"utf8")); const ok = v(d); const c={};
  for (const e of (v.errors||[])) { const k=e.instancePath.replace(/\/\d+/g,"/#").replace(/\/(attributes|schemas)\/[^/]+/,"/$1/*")+" "+e.keyword+" "+(e.params.missingProperty||e.params.additionalProperty||""); c[k]=(c[k]||0)+1; }
  const bad=(d.data||[]).filter(n=>!pat.test(n.path)).length;
  const meshTris=(d.data||[]).reduce((s,n)=>s+((n.attributes&&n.attributes['usd::usdgeom::mesh']?.faceVertexIndices?.length)||0)/3,0);
  const meshNodes=(d.data||[]).filter(n=>n.attributes&&n.attributes['usd::usdgeom::mesh']).length;
  console.log(JSON.stringify({meshTris,meshNodes,valid:ok, pathPatternViolations:bad, nodes:(d.data||[]).length, kinds:Object.fromEntries(Object.entries(c).sort((a,b)=>b[1]-a[1]).slice(0,10))})); }
