import sys, textwrap
import pathlib
ROOT = pathlib.Path(__file__).resolve().parents[4]
MOCK = ROOT / 'docs/design/tui-declutter/mockups'
HERE=pathlib.Path(__file__).resolve().parent
sys.path.insert(0,str(HERE))
src=open(HERE/'gen_screens.py').read().split('def landed')[0]
exec(src)                      # reuse helpers (S, render, vis, emit, tool, body, prose, bottom...)
OUT=str(MOCK/'dark/notices')+'/'
_save=save
def save(name):   # option A (a_*) is not kept in the repo
    global lines
    if name.startswith('a_'): lines=[]
    else: _save(name)
FLOAT_MAX=46; RIGHT=W          # float's last cell is column 99

def transcript():
    tool('●',GRN,'Edit','src/config.rs',EDIT_F)
    for l in EDIT[:4]: body(l,True)
    fold('… 3 more lines'); blank()
    tool('●',GRN,'Shell','cargo test -p hoocode-config 2>&1 | tail -40',D('exit 0 · 8.2s'))
    fold('… 38 earlier lines')
    for l in SHELL[-2:]: body(l)
    for t,c in CFG_PROSE: blank(); prose(t,c)

# ---- segment helpers for overlay -------------------------------------------------
import re
def parse(line):
    """inline «style» row -> list of (char, style) cells"""
    cells=[]; st=NONE
    for tok in re.split(r'(«[^»]*»)',line):
        if tok.startswith('«'):
            fg=bg=None; b=i=False
            for kv in filter(None,tok[1:-1].split(';')):
                k,_,v=kv.partition('=')
                if k=='fg': fg=v
                elif k=='bg': bg=v
                elif k=='bold': b=True
                elif k=='italic': i=True
            st=S(fg,bg,b,i)
        else: cells+= [(ch,st) for ch in tok]
    return cells
def overlay(row, segs):
    """put segs so they end at the right edge, over transcript row `row`"""
    cells=parse(lines[row]); w=vis(segs); start=W-w
    cells=(cells+[(' ',NONE)]*W)[:start]
    lines[row]=render([(c,s) for c,s in cells]+segs)

def note(key, action, bg=None):
    return [('  ',S(None,bg)),(key,S(TXT,bg)),(' '+action,S(DIM,bg))]

def wrap_rows(text, width, maxrows=3):
    # a notice is a list of parts (message, then hint); each part wraps on its own at spaces,
    # so a break never lands mid-phrase; past maxrows the last row ends in "…"
    parts=[text] if isinstance(text,str) else text
    if sum(len(x)+1 for x in parts)-1<=width: parts=[' '.join(parts)]
    rows=[r for x in parts for r in textwrap.wrap(x,width,break_long_words=True)]
    if len(rows)>maxrows:
        rows=rows[:maxrows]; rows[-1]=rows[-1][:width-1].rstrip()+'…'
    return rows

# ---- option A: float ------------------------------------------------------------
def float_rows(rows):
    """rows: list of segment lists (no padding, no bg). pad 1 each side, fill surface"""
    inner=min(max(vis(r) for r in rows),FLOAT_MAX-2)
    out=[]
    for r in rows:
        r=[(t,S(st[0],SURF,st[2],st[3])) for t,st in r]
        out.append([(' ',S(bg=SURF))]+r+[(' '*(inner-vis(r)+1),S(bg=SURF))])
    return out

def screen_a(name, rows, model=None):
    blank(); transcript(); blank(); bottom('peek','done')
    if model:  # status line shows the new model; take the extra width from the gap
        d=len(model)-len('claude-haiku-5-5'); lines[-1]=lines[-1].replace(' '*d+'«fg=#989898»peek','«fg=#989898»peek',1).replace('claude-haiku-5-5',model)
    for i,r in enumerate(float_rows(rows)): overlay(1+i,r)
    save(name)

INNER=FLOAT_MAX-2
# 47 cols > FLOAT_MAX: message and hint wrap as separate parts (wrap rule)
screen_a('a_info_undo.txt',[[('switched to claude-sonnet-5-5',S(TXT))],note('shift+alt+m','back')[1:]],'claude-sonnet-5-5')
screen_a('a_info_value.txt',[[('thinking level: high',S(TXT))]+note('shift+alt+t','back')])
title,bodyrows=('Sign in to start',["Run /login: a subscription or an API key.","Or set e.g. OPENAI_API_KEY and restart."])
screen_a('a_warning.txt',[[(title,S(YEL,None,True))]]+[[(b,S(TXT))] for b in bodyrows])
USAGE=["Usage: /copy [all|<number of turns>]"]
u=wrap_rows(USAGE,INNER)
screen_a('a_usage.txt',[[(u[0],S(YEL,None,True))]]+[[(x,S(TXT))] for x in u[1:]])

# ---- option B: status line ------------------------------------------------------
def bottom_notice(left, model='claude-haiku-5-5'):
    fill([(' › ',S(TEAL,SURF))])
    # same narrow-terminal drop order as the working state: cost, session name, 'context' word
    parts=['peek','amber-harbor',model,'42% context','$1.23']
    drops=[lambda p: p.remove('$1.23'), lambda p: p.remove('amber-harbor'),
           lambda p: p.__setitem__(p.index('42% context'),'42%')]
    right=' · '.join(parts)
    while vis(left)+2+len(right)>W-1 and drops:
        drops.pop(0)(parts); right=' · '.join(parts)
    assert vis(left)+2+len(right)<=W-1
    emit(left+[(' '*(W-1-vis(left)-len(right)),NONE),(right,S(DIM))])

def screen_b_slot(name, segs, model='claude-haiku-5-5'):
    blank(); transcript(); blank(); bottom_notice([(' ',NONE)]+segs, model); save(name)

def screen_b_rows(name, rows):
    blank(); transcript(); blank()
    for r in rows: emit([(' ',NONE)]+r)
    bottom('peek','done'); save(name)

screen_b_slot('b_info_undo.txt',[('switched to claude-sonnet-5-5',S(TXT))]+note('shift+alt+m','back'),'claude-sonnet-5-5')
screen_b_slot('b_info_value.txt',[('thinking level: high',S(TXT))]+note('shift+alt+t','back'))
screen_b_rows('b_warning.txt',[[(title,S(YEL,None,True))]]+[[(b,S(DIM))] for b in bodyrows])
ub=wrap_rows(USAGE,W-2)
screen_b_rows('b_usage.txt',[[(ub[0],S(YEL))]]+[[(x,S(DIM))] for x in ub[1:]])
