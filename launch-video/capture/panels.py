#!/usr/bin/python3.12
# Ubuntu 24.04 style top bar + left dock (GTK3 DOCK windows with struts)
import gi, subprocess, math, cairo
gi.require_version('Gtk','3.0'); gi.require_version('Gdk','3.0'); gi.require_version('PangoCairo','1.0'); gi.require_version('GdkX11','3.0')
from gi.repository import Gtk, Gdk, GLib, GdkPixbuf, Pango, PangoCairo, GdkX11
W,H=2560,1440
TB=42
ICON=64; SLOT=84; DW=96
Y='/usr/share/icons/Yaru/256x256@2x/apps/'
APPS=[('Files',Y+'org.gnome.Nautilus.png',('nautilus',)),('Chromium','/usr/share/icons/Papirus/64x64/apps/chromium.svg',('chrome',)),
 ('Terminal',Y+'org.gnome.Terminal.png',('gnome-terminal-','xfce4-terminal')),('Text Editor',Y+'org.gnome.TextEditor.png',('gnome-text-edit',)),
 ('Calculator',Y+'org.gnome.Calculator.png',('gnome-calculator',)),('Settings',Y+'org.gnome.Settings.png',('gnome-control-c',)),
 ('RustCast','/home/user/rustcast/assets/icons/rustcast-256.png',('rustcast',)),('Trash','/usr/share/icons/Yaru/256x256@2x/places/user-trash.png',())]
icons=[GdkPixbuf.Pixbuf.new_from_file_at_scale(a[1],ICON,ICON,True) for a in APPS]
def sym(name,size):
    import glob
    f=(glob.glob(f'/usr/share/icons/Yaru/scalable/*/{name}.svg')+[None])[0]
    return GdkPixbuf.Pixbuf.new_from_file_at_scale(f,size,size,True) if f else None
TRAY=[sym('network-wireless-signal-excellent-symbolic',22),sym('audio-volume-high-symbolic',22),sym('battery-full-symbolic',22),sym('system-shutdown-symbolic',22)]
GRID=sym('view-app-grid-ubuntu-symbolic',40) or sym('view-app-grid-symbolic',40)
RC=GdkPixbuf.Pixbuf.new_from_file_at_scale('/home/user/rustcast/assets/icons/rustcast-glyph-dark.png',24,24,True)
running=set(); active=''
def white(pb):
    # symbolic icons are dark grey; recolour to white keeping alpha
    pb=pb.copy(); n=pb.get_n_channels(); px=bytearray(pb.get_pixels()); rs=pb.get_rowstride()
    for y in range(pb.get_height()):
        for x in range(pb.get_width()):
            i=y*rs+x*n; px[i]=px[i+1]=px[i+2]=255
    return GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(bytes(px)),pb.get_colorspace(),True,8,pb.get_width(),pb.get_height(),rs)
TRAY=[white(t) for t in TRAY if t]; GRID=white(GRID)
def poll():
    global running,active
    try: running=set(subprocess.run(['ps','-eo','comm'],capture_output=True,text=True).stdout.split())
    except Exception: pass
    try:
        wid=subprocess.run(['xdotool','getactivewindow'],capture_output=True,text=True,timeout=1).stdout.strip()
        active=subprocess.run(['xprop','-id',wid,'WM_CLASS'],capture_output=True,text=True,timeout=1).stdout.lower() if wid else ''
    except Exception: active=''
    dock.queue_draw(); return True
def rrect(cr,x,y,w,h,r):
    cr.new_sub_path(); cr.arc(x+w-r,y+r,r,-math.pi/2,0); cr.arc(x+w-r,y+h-r,r,0,math.pi/2)
    cr.arc(x+r,y+h-r,r,math.pi/2,math.pi); cr.arc(x+r,y+r,r,math.pi,3*math.pi/2); cr.close_path()
def text(cr,s,x,y,size,weight='Medium',color=(1,1,1,1),anchor='l'):
    lay=PangoCairo.create_layout(cr); lay.set_font_description(Pango.FontDescription(f'Ubuntu Sans {weight} {size}px'))
    lay.set_text(s,-1); w,h=lay.get_pixel_size()
    if anchor=='r': x-=w
    if anchor=='c': x-=w/2
    cr.set_source_rgba(*color); cr.move_to(x,y-h/2); PangoCairo.show_layout(cr,lay); return w
def mkwin(x,y,w,h,strut):
    win=Gtk.Window(type=Gtk.WindowType.POPUP)
    win.set_visual(win.get_screen().get_rgba_visual()); win.set_app_paintable(True)
    win.set_default_size(w,h); win.move(x,y); win.set_decorated(False); win.set_keep_above(True); win.stick(); win.strut=strut
    return win
tb=mkwin(0,0,W,TB,[0,0,TB,0])
def draw_tb(w,cr):
    cr.set_operator(cairo.OPERATOR_SOURCE); cr.set_source_rgba(0,0,0,1); cr.paint(); cr.set_operator(cairo.OPERATOR_OVER)
    # workspace indicator (GNOME 46): active pill + dots, inside a subtle button
    rrect(cr,14,7,92,28,14); cr.set_source_rgba(1,1,1,0.0); cr.fill()
    rrect(cr,26,TB/2-5,38,10,5); cr.set_source_rgba(1,1,1,.95); cr.fill()
    for i in range(2):
        cr.arc(78+i*18,TB/2,5,0,7); cr.set_source_rgba(1,1,1,.45); cr.fill()
    text(cr,'Oct 6  09:41',W/2,TB/2,17,'Bold',anchor='c')
    # right: rustcast indicator + system status group
    x=W-22
    for pb in reversed(TRAY):
        x-=22; Gdk.cairo_set_source_pixbuf(cr,pb,x,(TB-22)/2); cr.paint(); x-=16
    x-=24; Gdk.cairo_set_source_pixbuf(cr,RC,x-24,(TB-24)/2); cr.paint()
tb.connect('draw',draw_tb)
dock=mkwin(0,TB,DW,H-TB,[DW,0,0,0])
def draw_dock(w,cr):
    cr.set_operator(cairo.OPERATOR_SOURCE); cr.set_source_rgba(0.09,0.09,0.09,0.86); cr.paint(); cr.set_operator(cairo.OPERATOR_OVER)
    y=12
    for a,ic in zip(APPS,icons):
        is_run=any(p in running for p in a[2]) if a[2] else False
        key=a[0].lower()
        is_act=('chrom' in active and key=='chromium') or ('nautilus' in active and key=='files') or ('terminal' in active and key=='terminal')
        if is_act:
            rrect(cr,8,y,DW-16,SLOT-4,14); cr.set_source_rgba(1,1,1,.12); cr.fill()
        Gdk.cairo_set_source_pixbuf(cr,ic,(DW-ICON)/2,y+(SLOT-4-ICON)/2); cr.paint()
        if is_run:
            cr.arc(4,y+(SLOT-4)/2,3.2,0,7); cr.set_source_rgba(0.914,0.329,0.125,1); cr.fill()
        y+=SLOT
    # show apps at bottom
    gy=H-TB-SLOT-6
    Gdk.cairo_set_source_pixbuf(cr,GRID,(DW-40)/2,gy+(SLOT-40)/2); cr.paint()
dock.connect('draw',draw_dock)
for wdg in (tb,dock):
    wdg.show_all()
GLib.timeout_add(400,poll); Gtk.main()
