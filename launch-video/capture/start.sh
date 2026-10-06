#!/bin/bash
export DISPLAY=:99 GTK_THEME=Yaru-dark XCURSOR_THEME=Yaru GSK_RENDERER=cairo
pkill -x rustcast; pkill -x picom; pkill -x openbox; pkill -x pcmanfm; pkill -x nautilus; pkill -x gnome-text-edit; pkill -x thunar; pkill -x chrome; pkill -x Xvfb; sleep 0.8
Xvfb :99 -screen 0 2560x1440x24 +extension Composite +extension RANDR -nolisten tcp -dpi 128 >/tmp/claude-0/lv/xvfb.log 2>&1 &
sleep 1
eval $(dbus-launch --sh-syntax); echo "export DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS GTK_THEME=Yaru-dark" > /tmp/claude-0/lv/dbus.env
echo "Xft.dpi: 128" | xrdb -merge
openbox >/tmp/claude-0/lv/ob.log 2>&1 &
sleep 0.5
picom --backend xrender --shadow --shadow-radius 30 --shadow-opacity 0.55 --shadow-offset-x -30 --shadow-offset-y -18 --no-fading-openclose --corner-radius 14 --rounded-corners-exclude "window_type = 'dock'" >/tmp/claude-0/lv/picom.log 2>&1 &
feh --bg-fill /tmp/claude-0/lv/wall.png
GDK_SCALE=1 python3.12 /tmp/claude-0/lv/panels.py >/tmp/claude-0/lv/panels.log 2>&1 &
sleep 0.5
WINIT_X11_SCALE_FACTOR=1.33 GDK_SCALE=2 /home/user/rustcast/target/release/rustcast >/tmp/claude-0/lv/rc.log 2>&1 &
sleep 3
