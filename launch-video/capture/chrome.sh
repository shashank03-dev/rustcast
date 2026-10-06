#!/bin/bash
# chrome.sh URL X Y W H TITLE
export DISPLAY=:99; source /tmp/claude-0/lv/dbus.env
/opt/pw-browsers/chromium-1194/chrome-linux/chrome --user-data-dir=/root/.chromium-demo --no-sandbox --test-type --no-first-run --no-default-browser-check --disable-gpu --force-device-scale-factor=1.33 --app="$1" >/dev/null 2>&1 &
sleep 3.5
W=$(xdotool search --onlyvisible --name "$6" | tail -1)
wmctrl -i -r $W -b remove,maximized_vert,maximized_horz
xdotool windowmove $W $2 $3 windowsize $W $4 $5
echo $W
