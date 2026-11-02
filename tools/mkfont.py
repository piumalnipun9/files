import sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools import subset

src, outdir = sys.argv[1], sys.argv[2]
names = sys.argv[3].split(",")
f = TTFont(src)
cmap = f.getBestCmap()
glyph2cp = {}
for cp, g in cmap.items():
    if cp >= 0xE000:
        glyph2cp.setdefault(g, cp)
# ligature table: first glyph -> list of ligatures
rev = {g: chr(c) for c, g in cmap.items() if c < 0x80}
lig = {}
for lk in f["GSUB"].table.LookupList.Lookup:
    for st in lk.SubTable:
        if st.LookupType == 7:
            st = st.ExtSubTable
        if getattr(st, "LookupType", None) != 4 and not hasattr(st, "ligatures"):
            continue
        for first, ligs in st.ligatures.items():
            for l in ligs:
                try:
                    word = rev[first] + "".join(rev[c] for c in l.Component)
                except KeyError:
                    continue
                lig.setdefault(word, l.LigGlyph)
out = {}
for n in names:
    g = lig.get(n)
    if g is None or g not in glyph2cp:
        print("MISSING", n, g, file=sys.stderr); continue
    out[n] = glyph2cp[g]
cps = sorted(set(out.values()))
for fill, fam in ((0, "Files Symbols"), (1, "Files Symbols Fill")):
    t = TTFont(src)
    t = instancer.instantiateVariableFont(t, {"FILL": fill, "wght": 400, "GRAD": 0, "opsz": 20})
    opts = subset.Options(); opts.layout_features = []; opts.name_IDs = ["*"]
    s = subset.Subsetter(opts); s.populate(unicodes=cps); s.subset(t)
    for rec in t["name"].names:
        if rec.nameID in (1, 16): rec.string = fam
        elif rec.nameID in (4,): rec.string = fam
        elif rec.nameID == 6: rec.string = fam.replace(" ", "")
        elif rec.nameID in (2, 17): rec.string = "Regular"
    t.save(f"{outdir}/{fam.lower().replace(' ', '-')}.ttf")
for n, cp in out.items():
    print(f"{n}={cp:x}")
