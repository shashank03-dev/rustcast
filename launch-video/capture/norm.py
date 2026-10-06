import subprocess, json, sys
name=sys.argv[1]; d='/tmp/claude-0/lv/takes/'
t0=float(subprocess.run(['ffprobe','-v','error','-select_streams','v','-show_entries','packet=pts_time','-read_intervals','%+#1','-of','csv=p=0',d+name+'.mkv'],capture_output=True,text=True).stdout.split()[0])
subprocess.run(['ffmpeg','-y','-loglevel','error','-i',d+name+'.mkv','-vf','setpts=PTS-STARTPTS','-fps_mode','cfr','-r','60','-c:v','libx264','-preset','medium','-crf','14','-pix_fmt','yuv420p',d+name+'.mp4'],check=True)
ev=json.load(open(d+name+'.json'))
for e in ev: e['t']=round(e['t']-t0,4)
dur=float(subprocess.run(['ffprobe','-v','error','-show_entries','format=duration','-of','csv=p=0',d+name+'.mp4'],capture_output=True,text=True).stdout)
json.dump({'t0':t0,'dur':dur,'ev':ev},open(d+name+'.ev.json','w'))
marks=[(e['name'],e['t']) for e in ev if e['k']=='mark']; print(name,'dur',round(dur,2),marks)
# contact sheet at marks
ts=[m[1]+0.4 for m in marks]+[dur-0.3]
for i,t in enumerate(ts):
    subprocess.run(['ffmpeg','-y','-loglevel','error','-ss',str(t),'-i',d+name+'.mp4','-frames:v','1','-vf','scale=640:-1',f'{d}{name}_s{i}.png'])
from PIL import Image
ims=[Image.open(f'{d}{name}_s{i}.png') for i in range(len(ts))]
w,h=ims[0].size; cols=3; rows=(len(ims)+cols-1)//cols
sheet=Image.new('RGB',(cols*w,rows*h))
for i,im in enumerate(ims): sheet.paste(im,((i%cols)*w,(i//cols)*h))
sheet.save(f'{d}{name}_sheet.png')
import os
for i in range(len(ts)): os.remove(f'{d}{name}_s{i}.png')
