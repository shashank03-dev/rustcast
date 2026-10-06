import librosa, numpy as np, json
y,sr=librosa.load('song.wav',sr=22050,mono=True)
dur=len(y)/sr
tempo,beats=librosa.beat.beat_track(y=y,sr=sr,units='time',tightness=120)
onset=librosa.onset.onset_strength(y=y,sr=sr)
rms=librosa.feature.rms(y=y,hop_length=512)[0]; t=librosa.frames_to_time(np.arange(len(rms)),sr=sr,hop_length=512)
# low-band energy (kick/808)
S=np.abs(librosa.stft(y,n_fft=2048,hop_length=512)); f=librosa.fft_frequencies(sr=sr,n_fft=2048)
low=S[f<150].sum(0); mid=S[(f>300)&(f<3000)].sum(0); hi=S[f>6000].sum(0)
def sec(a,step=1.0):
    out=[]
    for s in np.arange(0,dur,step):
        m=(t>=s)&(t<s+step); out.append(float(a[m].mean()) if m.any() else 0)
    return np.array(out)
R=sec(rms); L=sec(low); M=sec(mid); Hh=sec(hi)
R/=R.max(); L/=L.max(); M/=M.max(); Hh/=Hh.max()
print('dur',round(dur,2),'tempo',tempo, 'nbeats',len(beats))
print('first beats',np.round(beats[:12],3))
ib=np.diff(beats); print('median IBI',np.median(ib))
print('sec  rms  low  mid  hi')
for i in range(len(R)):
    bar=lambda v:'#'*int(v*20)
    print(f'{i:3d} {R[i]:.2f} {L[i]:.2f} {M[i]:.2f} {Hh[i]:.2f} |{bar(R[i]):20s}|{bar(L[i])}')
json.dump({'beats':beats.tolist(),'tempo':float(np.atleast_1d(tempo)[0]),'rms':R.tolist(),'low':L.tolist(),'mid':M.tolist(),'hi':Hh.tolist()},open('analysis.json','w'))
