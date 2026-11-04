import re,sys,glob
import pathlib
ROOT = pathlib.Path(__file__).resolve().parents[4]
MOCK = ROOT / 'docs/design/tui-declutter/mockups'
PAL={'#e5e5e7','#989898','#2a2a36','#5cc8bb','#b5bd68','#d58181','#f0c674'}
BANNED=set('◐⊘✻▸◈◆▲┊╰└━┃')
ok=True
def fail(m):
    global ok; ok=False; print('FAIL',m)
for f in sorted(glob.glob(str(MOCK/'dark/notices')+'/*.txt')):
    n=f.split('/')[-1]
    rows=open(f).read().split('\n')
    if rows[-1]=='': rows=rows[:-1]
    for i,l in enumerate(rows,1):
        plain=re.sub(r'«[^»]*»','',l); w=len(plain)
        for k in re.findall(r'«([^»]*)»',l):
            for kv in filter(None,k.split(';')):
                key,_,val=kv.partition('=')
                if key not in ('fg','bg','bold','italic','dim'): fail(f'{n}:{i} key {kv}')
                if val and val not in PAL: fail(f'{n}:{i} colour {val}')
                if key=='bg' and val!='#2a2a36': fail(f'{n}:{i} bg {val}')
        surf='bg=' in l
        if w>100: fail(f'{n}:{i} len {w}')
        if surf and w!=100: fail(f'{n}:{i} surface row len {w}')
        if BANNED & set(plain): fail(f'{n}:{i} banned glyph {BANNED & set(plain)}')
    if 'bg=' in rows[0]: fail(f'{n}: first row is a band')
    if n!='start.txt' and rows[0]!='': fail(f'{n}: transcript must open with one blank row')
    st=re.sub(r'«[^»]*»','',rows[-1]).rstrip()
    if not re.search(r'(peek|outline|full) · ',st): fail(f'{n}: status right side lacks view name')
    if 'bg=' not in rows[-2]: fail(f'{n}: prompt not second-to-last')
    if 'bg=' in rows[-1] or not rows[-1].strip(): fail(f'{n}: bad status line')
    if n!='start.txt' and '' in (rows[-3],rows[0]) and rows[-3]=='' and rows[-4]=='': fail(f'{n}: filler blanks')
    if re.search(r'\n\n\n',open(f).read()) and n!='start.txt': fail(f'{n}: double blank')
    print(f'{n}: {len(rows)} rows')
print('ALL OK' if ok else 'FAILURES'); sys.exit(0 if ok else 1)
