"""WCAG AA check for the app's colour tokens (R49.7).

Usage: python3 scripts/check_contrast.py frontend/src/theme.css
Reads the light (:root) and dark (:root[data-theme="dark"]) --ac-* tokens and
checks text, status and citation colours on every surface. Exits 1 on any
pair under its minimum (4.5:1 for text). Run it before changing a token.
"""
import re,sys
css=open(sys.argv[1]).read()
def block(sel):
    i=css.index(sel); j=css.index('}',i); return dict(re.findall(r'--ac-([\w-]+):\s*(#[0-9A-Fa-f]{6})',css[i:j]))
light=block(':root {'); dark={**light,**block(':root[data-theme="dark"]')}
def L(h):
    c=[int(h[k:k+2],16)/255 for k in (1,3,5)]
    c=[x/12.92 if x<=0.03928 else ((x+0.055)/1.055)**2.4 for x in c]
    return 0.2126*c[0]+0.7152*c[1]+0.0722*c[2]
def cr(a,b): a,b=L(a),L(b); return (max(a,b)+0.05)/(min(a,b)+0.05)
surfaces=['bg','bg-elevated','bg-card','bg-hover']
pairs=[(t,s,4.5) for t in ['text','text-muted','text-dim','cite','green','amber','red','blue','slate'] for s in surfaces]
pairs+=[('green-text','green-bg',4.5),('amber','amber-bg',4.5),('red-text','red-bg',4.5),('red','red-bg',4.5),('blue','blue-bg',4.5),('cite','cite-soft',4.5),('text','cite-soft',4.5),('on-accent','accent',4.5),('text','muted-bg',4.5),('text','amber-bg',4.5),('border','bg',1.3)]
bad=0
for name,pal in (('light',light),('dark',dark)):
    for f,b,m in pairs:
        if f not in pal or b not in pal: continue
        r=cr(pal[f],pal[b])
        if r<m: bad+=1; print(f'FAIL {name} {f} on {b}: {r:.2f} < {m}')
    print(name,'min text/card', min(cr(pal[t],pal['bg-card']) for t in ['text','text-muted','text-dim']).__round__(2), 'cite/card', round(cr(pal['cite'],pal['bg-card']),2), 'dim/hover', round(cr(pal['text-dim'],pal['bg-hover']),2))
print('failures:',bad)
sys.exit(1 if bad else 0)
