"""Light palette + WCAG contrast table."""
def lum(h):
    c=[int(h[i:i+2],16)/255 for i in (1,3,5)]
    c=[x/12.92 if x<=0.04045 else ((x+0.055)/1.055)**2.4 for x in c]
    return 0.2126*c[0]+0.7152*c[1]+0.0722*c[2]
def cr(a,b):
    a,b=lum(a),lum(b); return (max(a,b)+0.05)/(min(a,b)+0.05)
def mix(a,b,w):
    ca=[int(a[i:i+2],16) for i in (1,3,5)]; cb=[int(b[i:i+2],16) for i in (1,3,5)]
    return "#"+"".join(f"{round(w*x+(1-w)*y):02x}" for x,y in zip(ca,cb))
BASE="#f8f8fb"
TEXT="#2b2f36"; DIM="#555864"; TEAL="#00625c"; GREEN="#0a6b27"; RED="#b8111e"; YEL="#7a5200"
PANE="#e8e8f0"; DETAIL="#d3d7e9"; SEL="#c3dcdc"; INPUT="#dadbe6"
CHIP=TEAL; CHIPFG=BASE; WARN="#f0c674"; WARNFG="#2b2f36"
SURF={"base":BASE,"pane":PANE,"detail":DETAIL,"sel":SEL,"input":INPUT}
FGS={"text":TEXT,"dim":DIM,"teal":TEAL,"green":GREEN,"red":RED,"yellow":YEL}
if __name__=="__main__":
    print("sel",SEL)
    print(f"{'':8}"+"".join(f"{k:>8}" for k in FGS)+"  vs base  vs pane")
    for s,h in SURF.items():
        print(f"{s:8}"+"".join(f"{cr(f,h):8.2f}" for f in FGS.values())+f"{cr(h,BASE):9.2f}{cr(h,PANE):8.2f}")
    print("chip",cr(CHIPFG,CHIP),"warn",cr(WARNFG,WARN),"chip vs pane",cr(CHIP,PANE),"warn vs pane",cr(WARN,PANE))
    print("sel-input",cr(SEL,INPUT),"detail-input",cr(DETAIL,INPUT), "sel-detail",cr(SEL,DETAIL))
