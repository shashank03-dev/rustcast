import numpy as np, sys
from PIL import Image, ImageFilter
W,H=int(sys.argv[1]),int(sys.argv[2])
y,x=np.mgrid[0:H,0:W].astype(np.float32); x/=W; y/=H
# deep sky gradient
top=np.array([20,18,40])/255.; bot=np.array([60,22,30])/255.
img=top*(1-y[...,None])+bot*y[...,None]
def wave(y0,amp,freq,ph,col,soft,alpha):
    edge=y0+amp*np.sin(x*freq+ph)+amp*.35*np.sin(x*freq*2.3+ph*1.7)
    m=1/(1+np.exp(-(y-edge)/soft))
    shade=np.clip(1-(y-edge)*1.6,0.35,1)[...,None]
    return m[...,None],np.array(col)/255.*shade,alpha
layers=[(.42,.06,2.6,.4,(120,64,170),.004,.9),(.52,.07,2.1,2.0,(214,82,92),.004,.95),(.63,.06,2.9,4.1,(242,110,64),.004,.95),(.74,.05,2.4,1.1,(255,154,92),.004,.95),(.86,.04,3.2,3.3,(190,58,40),.004,.95)]
for l in layers:
    m,c,a=wave(*l)
    # highlight rim along wave edge
    img=img*(1-m*a)+c*m*a
# soft glow
d=((x-.75)**2*3+(y-.45)**2)
img+=np.exp(-d/.05)[...,None]*np.array([255,170,120])/255.*.18
img=np.clip(img,0,1)
im=Image.fromarray((img*255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(1.2))
a=np.asarray(im).astype(np.float32)/255.
a=np.clip(a+np.random.default_rng(1).normal(0,.006,a.shape),0,1)
Image.fromarray((a*255).astype(np.uint8)).save(sys.argv[3])
