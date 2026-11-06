import sys,json,collections
from pxr import Usd, UsdGeom, UsdValidation
f=sys.argv[1]; out={}
try:
    st=Usd.Stage.Open(f); out["opened"]=True
    meshes=[p for p in st.Traverse() if p.IsA(UsdGeom.Mesh)]; out["meshes"]=len(meshes)
    out["noNormals"]=sum(1 for p in meshes if not UsdGeom.Mesh(p).GetNormalsAttr().HasAuthoredValue() and not UsdGeom.PrimvarsAPI(p).GetPrimvar("normals"))
    out["triangles"]=sum(len(UsdGeom.Mesh(p).GetFaceVertexCountsAttr().Get() or []) for p in meshes)
    out["colours"]=len({tuple(UsdGeom.Gprim(p).GetDisplayColorAttr().Get()[0]) for p in meshes if UsdGeom.Gprim(p).GetDisplayColorAttr().Get()})
    reg=UsdValidation.ValidationRegistry(); vs=reg.GetOrLoadAllValidators(); out["validators"]=len(vs)
    ctx=UsdValidation.ValidationContext(vs); errs=ctx.Validate(st); c=collections.Counter()
    for e in errs: c[str(e.GetType()).split(".")[-1]+" "+str(e.GetName() if hasattr(e,"GetName") else "")[:80]+" "+str(e.GetErrors()[0].GetMessage() if hasattr(e,"GetErrors") and e.GetErrors() else "")[:80]]+=1
    out["errors"]=len(errs); out["kinds"]=dict(c.most_common(15))
except Exception as e: out["exception"]=repr(e)[:300]; out["api"]=[a for a in dir(UsdValidation) if not a.startswith("_")]
print(json.dumps(out))
