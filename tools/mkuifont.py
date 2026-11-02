"""Static Google Sans Regular/Medium (Latin + common punctuation) from the variable font."""
import sys
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer
from fontTools import subset

src, outdir = sys.argv[1], sys.argv[2]
for wght, style in ((400, "Regular"), (500, "Medium")):
    t = instancer.instantiateVariableFont(TTFont(src), {"wght": wght, "opsz": 17, "GRAD": 0})
    opts = subset.Options()
    opts.layout_features = ["kern", "liga", "calt", "ccmp", "locl", "mark", "mkmk", "tnum"]
    opts.name_IDs = ["*"]
    s = subset.Subsetter(opts)
    s.populate(unicodes=list(range(0x20, 0x250)) + list(range(0x2000, 0x2070)) + [0x20AC, 0x2122, 0x2190, 0x2192, 0x2026])
    s.subset(t)
    for rec in t["name"].names:
        if rec.nameID in (1, 16):
            rec.string = "Google Sans"
        elif rec.nameID in (2, 17):
            rec.string = style
        elif rec.nameID == 4:
            rec.string = f"Google Sans {style}"
        elif rec.nameID == 6:
            rec.string = f"GoogleSans-{style}"
    t["OS/2"].usWeightClass = wght
    t.save(f"{outdir}/GoogleSans-{style}.ttf")
