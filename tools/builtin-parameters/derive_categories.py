#!/usr/bin/env python3
"""Names of built-in categories, from Revit's own IFC export of public samples.

usage: derive_categories.py PAIRS.tsv OUT.tsv

PAIRS.tsv: one line per sample, `<Revit's IFC export>\t<header_categories output for the same file>`
(`cargo run --release --example header_categories -- model.rvt`: element id, BuiltInCategory id).
Revit's export writes each element's category name as the `Category` property of its `Other`
property set (and of its type's). An element's header names its category by id, so each
(category id, name) pair is a vote; the most voted name of an id is kept, with the votes.
OUT.tsv: id, name, votes, models, other names (name:votes;...)."""
import collections, re, sys

ENT = re.compile(r"^#(\d+)=\s*(\w+)\((.*)\);\s*$")


def split_args(s):
    o, d, cur, q = [], 0, [], False
    i = 0
    while i < len(s):
        c = s[i]
        if q:
            cur.append(c)
            if c == "'":
                if i + 1 < len(s) and s[i + 1] == "'":
                    cur.append("'")
                    i += 1
                else:
                    q = False
        elif c == "'":
            q = True
            cur.append(c)
        elif c == '(':
            d += 1
            cur.append(c)
        elif c == ')':
            d -= 1
            cur.append(c)
        elif c == ',' and d == 0:
            o.append(''.join(cur))
            cur = []
        else:
            cur.append(c)
        i += 1
    o.append(''.join(cur))
    return o


def unq(s):
    s = s.strip()
    if s.startswith("'") and s.endswith("'"):
        s = s[1:-1].replace("''", "'")
    return re.sub(r"\\X2\\([0-9A-F]+)\\X0\\", lambda m: ''.join(chr(int(m.group(1)[i:i + 4], 16)) for i in range(0, len(m.group(1)), 4)), s)


def refs(s):
    return [int(x) for x in re.findall(r"#(\d+)", s)]


def ifc_categories(path):
    """Tag (an element's id) -> its `Category` property of the `Other` set."""
    ents, tag_of = {}, {}
    with open(path, encoding='latin-1') as f:
        buf = ''
        for line in f:
            if not buf and not line.startswith('#'):
                continue
            buf += line.rstrip('\n')
            if not buf.rstrip().endswith(';'):
                continue
            m = ENT.match(buf.strip())
            buf = ''
            if not m:
                continue
            i, t, a = int(m.group(1)), m.group(2), m.group(3)
            if t in ('IFCPROPERTYSET', 'IFCPROPERTYSINGLEVALUE', 'IFCRELDEFINESBYPROPERTIES'):
                ents[i] = (t, a)
            elif not t.startswith(('IFCREL', 'IFCPROPERTY', 'IFCQUANT')):
                a2 = split_args(a)
                if len(a2) > 7 and unq(a2[7]).isdigit():
                    tag_of[i] = (unq(a2[7]), a2)
    def category_of(sid):
        t, a = ents.get(sid, ('', ''))
        if t != 'IFCPROPERTYSET':
            return None
        a = split_args(a)
        if unq(a[2]).removesuffix('(Type)') != 'Other':
            return None
        for r in refs(a[4]):
            pt, pa = ents.get(r, ('', ''))
            if pt == 'IFCPROPERTYSINGLEVALUE':
                pa = split_args(pa)
                if unq(pa[0]) == 'Category':
                    m = re.match(r"IFC\w+\('(.*)'\)$", pa[2].strip(), re.S)
                    return unq("'" + m.group(1) + "'") if m else None
        return None
    out = {}
    for i, (t, a) in ents.items():
        if t == 'IFCRELDEFINESBYPROPERTIES':
            a2 = split_args(a)
            c = category_of(refs(a2[5])[0])
            if c:
                for o in refs(a2[4]):
                    if o in tag_of:
                        out.setdefault(tag_of[o][0], c)
    for i, (tag, a2) in tag_of.items():
        if len(a2) > 5 and a2[5].strip().startswith('('):
            for sid in refs(a2[5]):
                c = category_of(sid)
                if c:
                    out.setdefault(tag, c)
    return out


votes = collections.defaultdict(collections.Counter)
models = collections.defaultdict(set)
for line in open(sys.argv[1]):
    ifc, cats = line.rstrip('\n').split('\t')
    names = ifc_categories(ifc)
    n = 0
    for l in open(cats):
        e, c = l.split()
        if e in names and int(c) < 0:
            votes[int(c)][names[e]] += 1
            models[int(c)].add(ifc)
            n += 1
    print(ifc, 'elements joined', n, flush=True)
with open(sys.argv[2], 'w', encoding='utf-8') as f:
    f.write('# id\tname\tvotes\tmodels\tother names\n')
    for i in sorted(votes):
        c = votes[i]
        (name, v), rest = c.most_common(1)[0], c.most_common()[1:]
        f.write(f"{i}\t{name}\t{v}\t{len(models[i])}\t{';'.join(f'{k}:{n}' for k, n in rest)}\n")
