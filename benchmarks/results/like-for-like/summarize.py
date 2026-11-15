import re,json,glob,sys,statistics as st,os
d=os.path.join(os.path.dirname(os.path.abspath(__file__)),"raw")
out={}
for f in sorted(glob.glob(d+"/stream-*-rust.txt")):
    m=re.search(r"stream-(\d+)-(\d+)-pair(\d+)-rust",f); h,k,p=m.groups(); key=f"{h}x{k}KB"
    r=open(f).read(); c=open(f[:-9]+"-react.txt").read()
    g=lambda pat,s=r: float(re.search(pat,s).group(1)) if re.search(pat,s) else None
    js=[l for l in c.splitlines() if l.startswith("{")]
    rr=dict(open_first_sync=g(r"import\+first sync ([\d.]+) ms"),open_fmt=g(r"like for like\) ([\d.]+) ms"),
      cpu=g(r"CPU while streaming (\d+) ms"),
      stall=None)
    mm=re.search(r"at the end \(?.*",r)
    rm=re.search(r"rss ([\d.]+) MiB, peak [\d.]+ MiB at the end",r); rr["mem"]=float(rm.group(1)) if rm else None
    sm=re.search(r"max ([\d.]+)(µs|ms|s)\b",r); 
    if sm: rr["stall"]=float(sm.group(1))*{"µs":.001,"ms":1,"s":1000}[sm.group(2)]
    rc=None
    if js:
        j=json.loads(js[0]); rc=dict(open=j["open_ms"],open_fmt=j["open_formatted_ms"],cpu=j["stream_process_cpu_ms"],rss=j["memory_end"]["rss_mib"],pss=j["memory_end"]["pss_mib"],stall=j["frame_max_ms"])
    out.setdefault(key,[]).append((int(p),rr,rc))
for key,v in out.items():
    print("==",key,len(v),"pairs")
    for p,rr,rc in v: print(p,{a:b for a,b in rr.items()},rc)
    def med(side,i):
        xs=[(x[1] if side==1 else x[2]).get(i) for x in v if (x[side] is not None)]
        xs=[x for x in xs if x is not None]
        return (round(st.median(xs),1),min(xs),max(xs),len(xs)) if xs else None
    print("MEDIANS rust:",{i:med(1,i) for i in ["open_first_sync","open_fmt","cpu","mem","stall"]})
    print("MEDIANS react:",{i:med(2,i) for i in ["open","open_fmt","cpu","rss","pss","stall"]})
