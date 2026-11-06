import sys,re,json,collections,ifcopenshell,ifcopenshell.validate as V
f=sys.argv[1]; l=V.json_logger(); V.validate(f,l); c=collections.Counter()
for s in l.statements: c[re.sub(r"#\d+","#",str(s.get("message","")))[:120]]+=1
m=ifcopenshell.open(f); prods=m.by_type("IfcProduct")
rep=collections.Counter(p.is_a() for p in prods if getattr(p,"Representation",None) is not None)
mats=len(m.by_type("IfcMaterial")); styles=len(m.by_type("IfcSurfaceStyle"))
print(json.dumps({"materials":mats,"styles":styles,"withRep":sum(rep.values()),"repByClass":dict(rep),"errors":len(l.statements),"kinds":dict(c.most_common(15)),"products":len([p for p in prods if p.is_a()!="IfcOpeningElement" or True]),"guids":len({p.GlobalId for p in prods})}))
