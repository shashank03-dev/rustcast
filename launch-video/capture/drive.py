# Human-like desktop driver + recorder with event log (cursor, clicks, keys, markers)
import subprocess, time, json, math, random, os, sys
from Xlib import X, display
from Xlib.ext import xtest
os.environ['DISPLAY']=':99'
D=None; ROOT=None
rng=random.Random(7)
class Take:
    def __init__(s,name):
        global D,ROOT
        D=display.Display(':99'); ROOT=D.screen().root
        s.name=name; s.ev=[]; s.pos=s._q()
    def _q(s):
        p=ROOT.query_pointer(); return (p.root_x,p.root_y)
    def start(s):
        s.ff=subprocess.Popen(['ffmpeg','-y','-loglevel','error','-f','x11grab','-draw_mouse','0','-framerate','60','-video_size','2560x1440','-i',':99',
            '-c:v','libx264','-preset','ultrafast','-crf','12','-copyts',f'/tmp/claude-0/lv/takes/{s.name}.mkv'],stdin=subprocess.PIPE)
        time.sleep(0.8); s.log('cursor',x=s.pos[0],y=s.pos[1])
    def stop(s,tail=0.6):
        time.sleep(tail); s.ff.stdin.write(b'q'); s.ff.stdin.flush(); s.ff.wait()
        json.dump(s.ev,open(f'/tmp/claude-0/lv/takes/{s.name}.json','w'))
    def log(s,kind,**kw):
        kw.update(t=time.time(),k=kind); s.ev.append(kw)
    def _set(s,x,y):
        xtest.fake_input(D,X.MotionNotify,x=int(round(x)),y=int(round(y))); D.sync(); s.pos=(x,y); s.log('cursor',x=x,y=y)
    def move(s,x,y,dur=None,arc=None,overshoot=True):
        x0,y0=s.pos; dx,dy=x-x0,y-y0; dist=math.hypot(dx,dy)
        if dist<2: return
        dur=dur or min(1.1,0.28+0.12*math.log2(1+dist/40))
        arc=arc if arc is not None else rng.uniform(-.12,.12)
        # control point perpendicular offset -> gentle curved path
        cx,cy=x0+dx*.5-dy*arc,y0+dy*.5+dx*arc
        ox,oy=(dx/dist*min(14,dist*.03),dy/dist*min(14,dist*.03)) if overshoot and dist>300 else (0,0)
        n=max(8,int(dur*120))
        for i in range(1,n+1):
            u=i/n; e=u*u*u*(10-15*u+6*u*u)  # minimum-jerk
            bx=(1-e)**2*x0+2*(1-e)*e*cx+e*e*(x+ox); by=(1-e)**2*y0+2*(1-e)*e*cy+e*e*(y+oy)
            s._set(bx,by); time.sleep(dur/n)
        if ox or oy:
            for i in range(1,13):
                u=i/12; e=u*u*(3-2*u); s._set(x+ox*(1-e),y+oy*(1-e)); time.sleep(0.011)
    def down(s,b=1):
        xtest.fake_input(D,X.ButtonPress,b); D.sync(); s.log('down',x=s.pos[0],y=s.pos[1])
    def up(s,b=1):
        xtest.fake_input(D,X.ButtonRelease,b); D.sync(); s.log('up',x=s.pos[0],y=s.pos[1])
    def click(s,x=None,y=None,pause=0.08):
        if x is not None: s.move(x,y); time.sleep(rng.uniform(.06,.14))
        s.down(); time.sleep(pause); s.up()
    def drag(s,x,y,dur=None):
        s.down(); time.sleep(0.12); s.move(x,y,dur=dur,overshoot=False); time.sleep(0.1); s.up()
    def key(s,combo,label=None):
        s.log('key',combo=combo,label=label or combo); subprocess.run(['xdotool','key','--clearmodifiers',combo])
    def type(s,text,wpm=1.0):
        for i,ch in enumerate(text):
            s.log('char',c=ch)
            subprocess.run(['xdotool','type','--delay','0',ch])
            d=rng.gauss(0.075,0.025)/wpm
            if ch==' ': d+=rng.uniform(0.02,0.09)
            if rng.random()<0.05: d+=rng.uniform(0.12,0.25)
            time.sleep(max(0.03,d))
    def mark(s,name,**kw): s.log('mark',name=name,**kw)
    def wait(s,t): time.sleep(t)
