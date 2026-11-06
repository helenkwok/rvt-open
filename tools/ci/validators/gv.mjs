import v from "gltf-validator"; import fs from "fs";
const p=process.argv[2]; const r=await v.validateBytes(new Uint8Array(fs.readFileSync(p)),{maxIssues:100000});
const c={}; for(const m of r.issues.messages){ if(m.severity<2){const k=(m.severity?"W ":"E ")+m.code;c[k]=(c[k]||0)+1} }
// normals: every primitive must have NORMAL
const j=JSON.parse(Buffer.from(fs.readFileSync(p)).subarray(20,20+fs.readFileSync(p).readUInt32LE(12)).toString());
let prims=0,noNormal=0; for(const m of j.meshes||[]) for(const q of m.primitives){prims++; if(!q.attributes.NORMAL) noNormal++}
const seen={}; let tris=0; for(const n of j.nodes||[]) if(n.mesh!==undefined){const t=n.extras?.ifcType??'?'; (seen[t]??=new Set()).add(n.extras?.entityIndex??Math.random()); for(const q of j.meshes[n.mesh].primitives){const ix=q.indices; tris+=(ix!==undefined?j.accessors[ix].count:j.accessors[q.attributes.POSITION].count)/3}}
const byType=Object.fromEntries(Object.entries(seen).map(([k,v])=>[k,v.size])); const materials=(j.materials||[]).length;
console.log(JSON.stringify({byType,triangles:tris,materials,errors:r.issues.numErrors,warnings:r.issues.numWarnings,kinds:c,primitives:prims,noNormal,meshNodes:(j.nodes||[]).filter(n=>n.mesh!==undefined).length}));
