"""Revit's own IFC export joined to the parameters a file stores, per element.

usage: ifcjoin.py IFC_DIR CENSUS_DIR OUT_DIR BIP_ENUM.tsv

IFC_DIR holds `<model>.ifc` (Revit's export with its property sets) and CENSUS_DIR `<model>.tsv.gz`
(`examples/param_census.rs` on the same sample as saved by the release that exported it, all origins;
a sample with no census file gives only its property sets, for the label route). For each model it writes OUT_DIR/<model>.props.tsv (set,
property, IFC value type, count of Revit's own property sets), OUT_DIR/<model>.cprops.tsv (the same
counted per element category: the `Category` property of the element's `Other` set) and OUT_DIR/<model>.pairs.tsv (an
element's IFC property with a parameter id the file stores on that element: `L` when the property's
name is the parameter's Revit 2027 API reference label, `V` when the value is equal). derive_ifc.py
reads them."""
import argparse,re,os,gzip,collections,json,csv
ap=argparse.ArgumentParser(description=__doc__,formatter_class=argparse.RawDescriptionHelpFormatter)
for n in ('ifc_dir','census_dir','out_dir','bip_enum'): ap.add_argument(n)
args=ap.parse_args()
ifcdir,cdir,out,bipf=args.ifc_dir,args.census_dir,args.out_dir,args.bip_enum
os.makedirs(out,exist_ok=True)
ENT=re.compile(r"^#(\d+)=\s*(\w+)\((.*)\);\s*$")
def split_args(s):
    o,d,cur,q=[],0,[],False
    i=0
    while i<len(s):
        c=s[i]
        if q:
            cur.append(c)
            if c=="'":
                if i+1<len(s) and s[i+1]=="'": cur.append("'"); i+=1
                else: q=False
        elif c=="'": q=True; cur.append(c)
        elif c=='(': d+=1; cur.append(c)
        elif c==')': d-=1; cur.append(c)
        elif c==',' and d==0: o.append(''.join(cur)); cur=[]
        else: cur.append(c)
        i+=1
    o.append(''.join(cur)); return o
def unq(s):
    s=s.strip()
    if s.startswith("'") and s.endswith("'"): s=s[1:-1].replace("''","'")
    s=re.sub(r"\\X2\\([0-9A-F]+)\\X0\\",lambda m:''.join(chr(int(m.group(1)[i:i+4],16)) for i in range(0,len(m.group(1)),4)),s)
    return s
def refs(s): return [int(x) for x in re.findall(r"#(\d+)",s)]
# public labels by id
lab=collections.defaultdict(set)
for l in open(bipf):
    if l[0]=='#': continue
    p=l.rstrip('\n').split('\t')
    if len(p)>2 and p[2]: lab[int(p[1])].add(p[2])
def cl(x): return re.sub(r'[\t\r\n]',' ',str(x))  # a tab or newline in a name or value must not move a column or row
NAMES=set()  # the model's own storey and type names
def ifc_props(path):
    ents={}
    keep=('IFCPROPERTYSET','IFCPROPERTYSINGLEVALUE','IFCRELDEFINESBYPROPERTIES')
    tag_of={}
    cls_other=collections.Counter()
    NAMES.clear()
    with open(path,encoding='latin-1') as f:
        buf=''
        for line in f:
            if not buf and not line.startswith('#'): continue
            buf+=line.rstrip('\n')
            if not buf.rstrip().endswith(';'): continue
            m=ENT.match(buf.strip()); buf=''
            if not m: continue
            i,t,a=int(m.group(1)),m.group(2),m.group(3)
            if t in keep: ents[i]=(t,a)
            elif t.startswith('IFCPROPERTY') and t!='IFCPROPERTYSET': cls_other[t]+=1
            elif not t.startswith('IFCREL') and not t.startswith('IFCPROPERTY') and not t.startswith('IFCQUANT'):
                a2=split_args(a)
                if (t=='IFCBUILDINGSTOREY' or t.endswith('TYPE')) and len(a2)>2:
                    NAMES.add(('storey' if t=='IFCBUILDINGSTOREY' else 'type',unq(a2[2])))
                if len(a2)>7:
                    tg=unq(a2[7])
                    if tg.isdigit():
                        tag_of[i]=(tg,a2)
    return ents,tag_of,cls_other
def pset(ents,i):
    t,a=ents.get(i,('',''))
    if t!='IFCPROPERTYSET': return None,[]
    a=split_args(a); name=unq(a[2]); props=[]
    for r in refs(a[4]):
        pt,pa=ents.get(r,('',''))
        if pt=='IFCPROPERTYSINGLEVALUE':
            pa=split_args(pa); props.append((unq(pa[0]),pa[2].strip()))
    return name,props
def parse_val(v):
    m=re.match(r"(IFC\w+)\((.*)\)$",v,re.S)
    if not m: return v,None
    t,x=m.groups()
    if x.startswith("'"): return t,unq(x)
    if x in ('.T.','.F.'): return t,1 if x=='.T.' else 0
    try: return t,float(x)
    except ValueError: return t,x
for fn in sorted(os.listdir(ifcdir)):
    if not fn.endswith('.ifc'): continue
    label=fn[:-4]
    cf=f'{cdir}/{label}.tsv.gz'
    has_census=os.path.exists(cf)
    if not has_census: print('no census',label,'(label route only)')
    ents,tag_of,cls_other=ifc_props(os.path.join(ifcdir,fn))
    with open(f'{out}/{label}.names.tsv','w') as nf:
        nf.write('#kind\tname\n')
        for kind,nm in sorted(NAMES):
            if nm: nf.write(f'{kind}\t{cl(nm)}\n')
    # element -> [(set,prop,type,val)]
    el=collections.defaultdict(list)
    def add(tag,sid):
        n,props=pset(ents,sid)
        if not n or n.startswith('Pset_') or n.startswith('Qto_'): return
        n=n[:-6] if n.endswith('(Type)') else n
        for pn,pv in props:
            t,v=parse_val(pv); el[tag].append((n,pn,t,v))
    for i,(t,a) in ents.items():
        if t=='IFCRELDEFINESBYPROPERTIES':
            a2=split_args(a); objs=refs(a2[4]); sid=refs(a2[5])[0]
            for o in objs:
                if o in tag_of: add(tag_of[o][0],sid)
    for i,(tg,a2) in tag_of.items():
        if len(a2)>5 and a2[5].strip().startswith('('):
            for sid in refs(a2[5]): add(tg,sid)
    pc=collections.Counter()
    for tag,props in el.items():
        for (st,pn,ity,iv) in props: pc[(st,pn,ity)]+=1
    cp=collections.Counter()
    for tag,props in el.items():
        cat=next((iv for (st,pn,ity,iv) in props if st=='Other' and pn=='Category' and isinstance(iv,str)),None)
        if cat:
            for (st,pn,ity,iv) in props: cp[(cat,st,pn,ity)]+=1
    with open(f'{out}/{label}.cprops.tsv','w') as cf_:
        cf_.write('#category\tset\tprop\tifctype\tn\n')
        for (cat,st,pn,ity),n in cp.items(): cf_.write(f'{cl(cat)}\t{cl(st)}\t{cl(pn)}\t{cl(ity)}\t{n}\n')
    with open(f'{out}/{label}.props.tsv','w') as pf:
        pf.write('#set\tprop\tifctype\tn\n')
        for (st,pn,ity),n in pc.items(): pf.write(f'{cl(st)}\t{cl(pn)}\t{cl(ity)}\t{n}\n')
    file=collections.defaultdict(list)
    if has_census:
      with gzip.open(cf,'rt',encoding='utf-8',errors='replace') as g:
        for line in g:
            p=line.rstrip('\n').split('\t')
            if len(p)<5: continue
            file[p[0]].append((int(p[1]),p[2],p[3],p[4]))
    FAC=[1.0,304.8,0.09290304,0.028316846592,57.29577951308232,0.3048,1000.0*0.3048,0.0929,0.0283168]
    def eq(fv,t,iv,ity):
        if iv is None: return False
        if t=='String': return isinstance(iv,str) and fv==iv
        try: x=float(fv)
        except ValueError: return False
        if isinstance(iv,str): return False
        for f in FAC:
            if abs(x*f-iv)<=1e-6*max(1,abs(iv)) or abs(x*f-iv)<=0.002*abs(iv)+1e-9: return True
        return False
    o=open(f'{out}/{label}.pairs.tsv','w')
    o.write('#tag\tset\tprop\tifctype\tifcvalue\tparam_id\torigin\tfiletype\tfilevalue\tmethod\n')
    nprop=npair=0
    for tag,props in el.items():
        fp=file.get(tag,[])
        for (st,pn,ity,iv) in props:
            nprop+=1
            hit=False
            for (pid,org,ft,fv) in fp:
                meth=None
                if pn in lab.get(pid,()): meth='L'
                if eq(fv,ft,iv,ity) and ft!='ElementId' and not (ft=='Double' and abs(float(fv))<1e-9) : meth=(meth or '')+'V'
                if meth:
                    hit=True; npair+=1
                    o.write(f'{tag}\t{cl(st)}\t{cl(pn)}\t{cl(ity)}\t{cl(iv)}\t{pid}\t{org}\t{ft}\t{cl(fv)}\t{meth}\n')
    o.close()
    print(label,'ifc elements',len(el),'props',nprop,'pairs',npair,'other classes',dict(cls_other.most_common(4)),flush=True)
