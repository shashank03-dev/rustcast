import sys, time, subprocess, os
sys.path.insert(0,'/tmp/claude-0/lv')
from drive import Take
def sh(c): subprocess.run(c,shell=True,executable='/bin/bash')
def reset(dash=True, code=False):
    sh('/tmp/claude-0/lv/start.sh')
    if code: sh('/tmp/claude-0/lv/chrome.sh file:///root/code/demo/code.html 380 200 1800 940 "search.rs" >/dev/null')
    sh('export DISPLAY=:99 GSK_RENDERER=cairo GDK_SCALE=1; source /tmp/claude-0/lv/dbus.env; (nautilus --gapplication-service >/dev/null 2>&1 &)')
    if dash: sh('/tmp/claude-0/lv/chrome.sh file:///root/code/demo/dashboard.html 330 150 1900 980 "Launch Metrics" >/dev/null')
    time.sleep(1.5)
    sh('DISPLAY=:99 xdotool mousemove 1800 1000'); time.sleep(0.3)

def launcher():
    reset()
    t=Take('launcher'); t.start(); t.wait(0.7)
    t.move(1640,760); t.wait(0.3)
    t.key('alt+space','Alt Space'); t.mark('open'); t.wait(0.9)
    t.type('chro'); t.mark('apps'); t.wait(1.1)
    for _ in range(4): t.key('BackSpace','⌫'); t.wait(0.06)
    t.wait(0.3); t.type('1280 * 3 / 4'); t.mark('calc'); t.wait(1.3)
    t.key('Return','↵'); t.mark('copied'); t.wait(0.8)
    t.key('alt+space','Alt Space'); t.wait(0.9); t.type('capture'); t.mark('commands'); t.wait(1.4)
    t.key('Escape','esc'); t.stop(0.8)

def jev():
    reset()
    t=Take('jev'); t.start(); t.wait(0.6)
    t.key('alt+space','Alt Space'); t.wait(0.9)
    t.type('jev open downloads then tile left',wpm=1.1); t.mark('typed'); t.wait(1.6)
    t.key('Return','↵'); t.mark('run'); t.wait(10.5)
    t.stop(0.5)


def snap():
    reset()
    t=Take('snap'); t.start(); t.wait(0.5)
    t.move(1650,1180); t.wait(0.3)
    t.key('super+shift+s','Super Shift S'); t.mark('freeze'); t.wait(1.1)
    t.move(1478,478); t.wait(0.25); t.mark('select')
    t.drag(2190,1032,dur=0.95); t.wait(0.6)
    t.key('a','A'); t.mark('arrow'); t.wait(0.25)
    t.move(1790,548); t.wait(0.15); t.drag(1930,612,dur=0.45); t.wait(0.4)
    t.key('n','N'); t.mark('steps'); t.wait(0.2)
    t.click(2150,624); t.wait(0.35); t.click(2150,772); t.wait(0.4)
    t.key('h','H'); t.mark('spot'); t.wait(0.2)
    t.move(1505,748); t.wait(0.15); t.drag(2166,798,dur=0.6); t.wait(0.6)
    t.key('ctrl+b','Ctrl B'); t.mark('beautify'); t.wait(1.0)
    t.key('Return','↵'); t.mark('done'); t.wait(1.4)
    t.move(330,1250); t.mark('thumb'); t.wait(2.2)
    t.stop(0.3)

if __name__=='__main__':
    globals()[sys.argv[1]]()
