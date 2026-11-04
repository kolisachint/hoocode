import textwrap
import pathlib
ROOT = pathlib.Path(__file__).resolve().parents[4]
MOCK = ROOT / 'docs/design/tui-declutter/mockups'
G=str(ROOT/'tests/golden/hoocode-code-tui-app')+'/'
OUT=str(MOCK/'dark/screens')+'/'
W=100
SURF='#2a2a36'; TXT='#e5e5e7'; DIM='#989898'
TEAL='#5cc8bb'; GRN='#b5bd68'; RED='#d58181'; YEL='#f0c674'

def S(fg=None,bg=None,bold=False,italic=False): return (fg,bg,bold,italic)
NONE=S()
def mk(st):
    fg,bg,b,i=st; k=[]
    if fg: k.append('fg='+fg)
    if bg: k.append('bg='+bg)
    if b: k.append('bold')
    if i: k.append('italic')
    return '«'+';'.join(k)+'»'
def render(segs):
    out=''; cur=NONE
    for t,st in segs:
        if not t: continue
        if st!=cur: out+= mk(st) if st!=NONE else '«»'; cur=st
        out+=t
    if cur!=NONE: out+='«»'
    return out
def vis(segs): return sum(len(t) for t,_ in segs)

lines=[]
def emit(segs):
    assert vis(segs)<=W, segs
    lines.append(render(segs))
def blank(): lines.append('')
def fill(segs):  # surface row, full width
    emit(segs+[(' '*(W-vis(segs)),S(bg=SURF))])

def user(paras):
    blank()
    first=True
    for p in paras:
        for wl in textwrap.wrap(p,W-4):
            fill([(' › ' if first else '   ',S(TEAL,SURF) if first else S(bg=SURF)),(wl,S(TXT,SURF))]); first=False

def bottom(view, last=None, working=False):
    fill([(' › ',S(TEAL,SURF))])
    if working: left=[(' ',NONE),('◉',S(YEL)),(' working · 4s · esc stop',S(DIM))]
    elif last=='done': left=[(' ',NONE),('●',S(GRN)),(' done · 11s · 23k tokens',S(DIM))]
    elif last=='failed': left=[(' ',NONE),('⊗',S(RED)),(' failed · 5.0s · 14k tokens',S(DIM))]
    elif last=='done1': left=[(' ',NONE),('●',S(GRN)),(' done · 8.5s · 7.1k tokens · ',S(DIM)),('1 failed call',S(RED))]
    else: left=[]
    # narrow drop order: cost, session name, 'context' word, model effort
    parts=[view,'amber-harbor','claude-haiku-5-5','42% context','$1.23']
    drops=[lambda p: p.remove('$1.23'), lambda p: p.remove('amber-harbor'),
           lambda p: p.__setitem__(p.index('42% context'),'42%')]
    right=' · '.join(parts)
    while vis(left)+2+len(right)>W-1 and drops:
        drops.pop(0)(parts); right=' · '.join(parts)
    emit(left+[(' '*(W-1-vis(left)-len(right)),NONE),(right,S(DIM))])

def tool(glyph,col,name,subject,facts=(),indent=' '):
    emit([(indent,NONE),(glyph,S(col)),(' ',NONE),(name,S(TXT,None,True)),(' '+subject,S(TXT))]
         +([('  ',NONE)]+list(facts) if facts else []))
def body(s,sign=False):
    if sign and s[0] in '-+':
        emit([('    ',NONE),(s[0],S(GRN if s[0]=='+' else RED)),(s[1:],S(DIM))])
    else: emit([('    '+s,S(DIM))])
def fold(s): emit([('    '+s,S(DIM))])

def prose(text, code=()):
    for wl in textwrap.wrap(text,W-2):
        segs=[(' ',NONE)]; rest=wl
        while rest:
            hits=[(rest.find(c),c) for c in code if rest.find(c)>=0]
            if not hits: segs.append((rest,S(TXT))); break
            i,c=min(hits)
            segs+= [(rest[:i],S(TXT)),(c,S(TEAL))]; rest=rest[i+len(c):]
        emit(segs)
def ended_failed(dur): emit([(' ',NONE),('⊗',S(RED)),(' ',NONE),('failed',S(RED)),(' · '+dur,S(DIM))])
def thinking(t,full):
    if full:
        for wl in textwrap.wrap(t,W-2): emit([(' '+wl,S(DIM,None,False,True))])
    else:
        emit([(' '+t.split('. ')[0]+'…',S(DIM,None,False,True))])
def save(name):
    global lines
    while lines and lines[-1]=='' and False: pass
    open(OUT+name,'w').write('\n'.join(lines)+'\n'); lines=[]
def src(f): return open(G+f).read().split('--- styles ---')[0].splitlines()

CFG_USER=["Make the config loader apply environment overrides before the file is parsed, so an override can set the parser's own options. Keep the public API the same for every caller in the workspace.",
 "Update the configuration section of the README so the order of resolution matches the code.",
 "Run the config tests afterwards and report anything that fails."]
THINK="The loader parses the file first and reads the environment after, so an override arrives too late. The file has to be read before I change anything."
CFG_PROSE=[("The overrides now apply before the file is parsed, so an override can set the parser's own options.",()),
 ("The public API is unchanged. Config::load keeps its signature, and the two callers in hoocode-cli need no edits.",('Config::load','hoocode-cli')),
 ("The config tests pass: 40 of 40. The README's resolution order now reads environment first, then the file, then the defaults.",())]
full=src('layers_landed_full_80.txt')
READ=[l[14:] for l in full[14:44]]          # gutter dropped
EDIT=[l[8:] for l in full[46:53]]
SHELL=[l[8:] for l in full[56:96]]
assert len(READ)==30 and len(EDIT)==7 and len(SHELL)==40
D=lambda s: [(s,S(DIM))]
EDIT_F=[('+2',S(GRN)),(' ',NONE),('−2',S(RED))]

def landed(view):
    user(CFG_USER); blank()
    if view=='outline':
        tool('●',GRN,'Read','src/config.rs · Edit src/config.rs · Shell cargo test')
    else:
        thinking(THINK, view=='full'); blank()
        tool('●',GRN,'Read','src/config.rs',D('30 lines'))
        if view=='full':
            for l in READ: body(l)
        if view=='full': blank()
        tool('●',GRN,'Edit','src/config.rs',EDIT_F)
        for l in (EDIT if view=='full' else EDIT[:4]): body(l,True)
        if view=='peek': fold('… 3 more lines')
        blank()
        tool('●',GRN,'Shell','cargo test -p hoocode-config 2>&1 | tail -40',D('exit 0 · 8.2s'))
        if view=='peek': fold('… 38 earlier lines')
        for l in (SHELL if view=='full' else SHELL[-2:]): body(l)
    for t,c in CFG_PROSE: blank(); prose(t,c)
    blank(); bottom(view,'done')

# start: 2 + 8 blank + prompt + status = 12
emit([(' hoocode v0.2.31',S(TXT,None,True))])
emit([(' /work/hoocode · 1 skill · 5 agents',S(DIM))])
for _ in range(8): blank()
bottom('peek'); save('start.txt')

for v in ('peek','outline','full'): landed(v); save(v+'.txt')

# failed (peek)
user(["The parser test fails on the escaped-quote case. Find the cause, fix it, and check the screenshot of the failing output too."]); blank()
tool('⊗',RED,'Shell','cargo test -p hoocode-parser escaped_quote',[('exit 1',S(RED)),(' · 3.1s',S(DIM))])
fold('… 6 earlier lines')
body('error: could not compile `hoocode-parser` (lib) due to 1 previous error')
blank()
tool('●',GRN,'Read','docs/screens/parser-failure.png',D('2 lines'))
blank()
prose("The escaped-quote case still fails. The lexer builds the token from a &str slice, but the parser expects an owned String, so the fix has to change the token type in lex.rs before the test can compile.",('&str','String','lex.rs'))
blank(); prose("I have not changed any file yet.")
blank(); emit([(' ',NONE),('⊗',S(RED)),(' connection reset by peer',S(DIM))])
blank(); ended_failed('5.0s')
blank(); bottom('peek','failed'); save('failed.txt')

# agent (outline)
user(["Find where the parser handles escaped quotes in string literals."]); blank()
tool('●',GRN,'Agent','explore · Trace escaped quotes')
emit([('   ',NONE),('⊗',S(RED)),(' ',NONE),('Grep',S(RED,None,True)),(' \\\\"  regex parse error: unclosed group',S(RED))])
blank()
prose("The escaped-quote handling lives in read_string() in src/parser/lex.rs. The subagent's search with the first pattern failed, but the read of the lexer and the token type answered the question.",('read_string()','src/parser/lex.rs'))
blank(); bottom('outline','done1'); save('agent.txt')

# working
user(["first question"]); blank()
tool('●',DIM,'Shell','cargo test -p hoocode-config')
blank()
emit([(' ○ steer this way · sends after this step',S(DIM))])
emit([(' ○ and then this · sends after this turn',S(DIM))])
bottom('peek',working=True); save('working.txt')
