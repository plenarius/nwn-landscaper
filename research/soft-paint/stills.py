# Off-screen client stills (throwaway, from canvas-halo's shoot3/stills.py): Xvfb with Ubuntu 24.04's Mesa 24.0 swrast (canvas-halo's
# xroot/old), the client on the system's llvmpipe.   python stills.py OUTDIR
import os, subprocess, sys, time
S = "/srv/burst-out/soft-paint"; C = f"{S}/client"; OUT = sys.argv[1]; os.makedirs(OUT, exist_ok=True)
G = "/srv/layonara-forge/game"; OLD = "/srv/burst-out/canvas-halo/xroot/old/usr/lib/x86_64-linux-gnu"
n = 83
while os.path.exists(f"/tmp/.X11-unix/X{n}") or os.path.exists(f"/tmp/.X{n}-lock"): n += 1
disp = f":{n}"
xvfb = subprocess.Popen(["Xvfb", disp, "-screen", "0", "1280x720x24", "-nolisten", "tcp"],
                        env=dict(os.environ, LD_LIBRARY_PATH=OLD, LIBGL_DRIVERS_PATH=f"{OLD}/dri"), stdout=open(f"{S}/logs/xvfb.log", "w"), stderr=subprocess.STDOUT)
time.sleep(3)
log = f"{C}/user/logs/nwclientLog1.txt"
if os.path.exists(log): os.remove(log)
env = dict(DISPLAY=disp, HOME=f"{C}/home", XDG_RUNTIME_DIR=f"{C}/run", SDL_AUDIODRIVER="dummy", ALSOFT_DRIVERS="null",
           LIBGL_ALWAYS_SOFTWARE="1", GALLIUM_DRIVER="llvmpipe", __GLX_VENDOR_LIBRARY_NAME="mesa", LP_NUM_THREADS="12",
           PATH="/usr/bin:/bin", LANG="C.UTF-8")
client = subprocess.Popen(["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=8G", "env", "-i"]
                          + [f"{k}={v}" for k, v in env.items()] + ["./nwmain-linux", "-userdirectory", f"{C}/user", "+TestNewModule", "sp_shoot"],
                          cwd=f"{G}/bin/linux-x86", stdout=open(f"{S}/logs/client.out", "w"), stderr=subprocess.STDOUT)
seen, shots, start = 0, 0, time.time()
def grab(name): subprocess.run([f"{S}/work/.venv/bin/python", "-c", "import sys\nfrom PIL import ImageGrab\nImageGrab.grab(xdisplay=sys.argv[1]).save(sys.argv[2])", disp, f"{OUT}/{name}.png"], check=False)
try:
    booted = False
    while time.time() - start < 2400 and client.poll() is None:
        time.sleep(1)
        if not booted and time.time() - start > 60: grab("_boot"); booted = True
        if seen == 0 and time.time() - start > 300: print("no shot after 300 s: hung start", flush=True); raise SystemExit(3)
        if not os.path.exists(log): continue
        lines = [l for l in open(log, errors="replace") if "ETSHOT " in l]
        for l in lines[seen:]:
            name = l.split("ETSHOT ", 1)[1].strip(); seen += 1
            if name == "done": raise SystemExit
            time.sleep(5); grab(name); shots += 1
            print(f"{time.time() - start:6.0f}s {name}", flush=True)
finally:
    grab("_last"); client.terminate(); time.sleep(2); client.kill(); xvfb.kill()
    subprocess.run(["pkill", "-f", f"userdirectory {C}/user"])
    print("shots", shots, "elapsed", round(time.time() - start))
