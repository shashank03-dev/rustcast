import sys,time,subprocess
sys.path.insert(0,'/tmp/claude-0/lv')
from takes import reset, sh, Take
X='DISPLAY=:99 '
def chrome(url,x,y,w,h,title): sh(f'/tmp/claude-0/lv/chrome.sh {url} {x} {y} {w} {h} "{title}" >/dev/null')
def geom(name):
    out=subprocess.run(f'{X}xdotool search --onlyvisible --name "{name}" | tail -1 | xargs -I{{}} sh -c "{X}xdotool getwindowgeometry --shell {{}}"',shell=True,capture_output=True,text=True).stdout
    d=dict(l.split('=') for l in out.split())
    return int(d['X']),int(d['Y']),int(d['WIDTH']),int(d['HEIGHT'])

def jev2():
    reset(); chrome('file:///root/code/demo/notes.html',260,200,1500,640,'Launch notes')
    sh(X+'xdotool search --name "Launch Metrics" | head -1 | xargs -I{} xdotool windowactivate {}'); time.sleep(0.8)
    t=Take('jev2'); t.start(); t.wait(0.5)
    t.key('alt+space','Alt Space'); t.wait(0.9)
    for i,q in enumerate(['jev create folder Launch on desktop','jev screenshot chromium in 5 seconds','jev record chromium','jev open downloads and chromium then show desktop']):
        if i: t.key('ctrl+a','Ctrl A'); t.key('BackSpace','⌫'); t.wait(0.4)
        t.type(q,wpm=1.5); t.mark(f'q{i}'); t.wait(1.6)
    t.stop(0.3)

def emoji():
    reset()
    t=Take('emoji'); t.start(); t.wait(0.5)
    t.key('alt+space','Alt Space'); t.wait(0.9)
    t.type('rocket'); t.mark('emoji'); t.wait(1.2)
    t.key('ctrl+a','Ctrl A'); t.key('BackSpace','⌫'); t.wait(0.4); t.type('fire'); t.mark('fire'); t.wait(1.2)
    t.key('Return','↵'); t.mark('copied'); t.wait(0.8)
    t.stop(0.3)

def palette():
    reset()
    t=Take('palette'); t.start(); t.wait(0.5)
    t.key('alt+space','Alt Space'); t.wait(0.9)
    t.type('palette'); t.wait(0.7); t.key('Return','↵'); t.mark('pick'); t.wait(1.2)
    t.move(2250,500); t.wait(0.2); t.mark('select'); t.drag(2545,1330,dur=0.8); t.mark('result'); t.wait(2.2)
    x,y,w,h=geom('Colours'); t.mark('win',x=x,y=y,w=w,h=h)
    by=y+h-68
    for rx in (0.166,0.253):
        t.click(x+int(rx*w),by); t.wait(0.7)
    t.click(x+int(0.756*w),by); t.mark('css'); t.wait(1.0)
    t.stop(0.3)

def compare():
    reset()
    def cap():
        sh(X+'xdotool key super+shift+s'); time.sleep(1.5)
        sh(X+'xdotool mousemove 420 260 mousedown 1'); 
        for i in range(1,26): sh(f'{X}xdotool mousemove {420+i*72} {260+i*24}'); time.sleep(0.02)
        sh(X+'xdotool mouseup 1'); time.sleep(1); sh(X+'xdotool key Return'); time.sleep(1.5)
    cap()
    chrome('file:///root/code/demo/dashboard2.html',330,150,1900,980,'Launch Metrics'); time.sleep(1)
    cap(); sh(X+'xdotool mousemove 1300 1350'); time.sleep(11)
    t=Take('compare'); t.start(); t.wait(0.5)
    t.key('alt+space','Alt Space'); t.wait(0.9); t.type('compare'); t.wait(0.6); t.key('Return','↵'); t.mark('open'); t.wait(2.0)
    x,y,w,h=geom('Compare'); t.mark('win',x=x,y=y,w=w,h=h)
    hx,hy=x+w//2,y+int(h*0.52)
    t.move(hx,hy); t.wait(0.25); t.mark('slide'); t.down(); t.wait(0.1)
    t.move(x+int(w*0.18),hy,dur=0.7,overshoot=False); t.wait(0.15); t.move(x+int(w*0.82),hy,dur=0.9,overshoot=False); t.wait(0.15); t.move(hx,hy,dur=0.5,overshoot=False); t.up(); t.wait(0.4)
    t.click(x+int(w*0.24),y+int(h*0.09)); t.mark('side'); t.wait(1.3)
    t.click(x+int(w*0.33),y+int(h*0.09)); t.mark('diff'); t.wait(2.0)
    t.stop(0.3)

def smart():
    reset(); chrome('file:///root/code/demo/notes.html',260,200,1500,640,'Launch notes'); time.sleep(0.5)
    t=Take('smart'); t.start(); t.wait(0.5)
    t.key('super+shift+t','Super Shift T'); t.wait(1.1)
    t.move(335,345); t.wait(0.2); t.mark('select'); t.drag(1712,700,dur=0.9); t.mark('ocr'); t.wait(2.6)
    x,y,w,h=geom('Text from Screen'); t.mark('win',x=x,y=y,w=w,h=h)
    t.wait(0.4)
    t.stop(0.3)

def table():
    reset()
    t=Take('table'); t.start(); t.wait(0.5)
    t.key('super+shift+t','Super Shift T'); t.wait(1.1)
    t.move(1505,560); t.wait(0.2); t.mark('select'); t.drag(2160,860,dur=0.8); t.mark('ocr'); t.wait(2.4)
    x,y,w,h=geom('Text from Screen'); t.mark('win',x=x,y=y,w=w,h=h)
    t.click(x+int(w*0.262),y+int(h*0.13)); t.mark('table'); t.wait(2.0)
    t.stop(0.3)

def pin():
    reset()
    t=Take('pin'); t.start(); t.wait(0.5)
    t.key('super+shift+s','Super Shift S'); t.wait(1.1)
    t.move(410,300); t.wait(0.2); t.mark('select'); t.drag(2240,480,dur=0.9); t.wait(0.5)
    t.key('b','B'); t.mark('censor'); t.move(440,370); t.wait(0.15); t.drag(820,440,dur=0.5); t.wait(0.5)
    t.key('ctrl+p','Ctrl P'); t.mark('pin'); t.wait(1.6)
    t.move(1200,1250); t.wait(0.3); t.mark('dragpin'); t.drag(1150,760,dur=0.9); t.wait(1.2)
    t.stop(0.3)
if __name__=='__main__': globals()[sys.argv[1]]()
