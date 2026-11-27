# mkmod.py ORDER: the stills module sp_shoot from the variants' exported modules (areas spa.., haks sp?.hak + ls_textures).
import json, os, shutil, subprocess, sys
S = "/srv/burst-out/soft-paint"; X = f"{S}/shoot/x"; SRC = f"{S}/shoot/src"; order = sys.argv[1]
def gff(p): subprocess.run(["nwn_gff", "-i", p, "-o", p + ".json"], check=True); x = json.load(open(p + ".json")); os.remove(p + ".json"); return x
def put(o, p): json.dump(o, open(p + ".json", "w")); subprocess.run(["nwn_gff", "-i", p + ".json", "-o", p], check=True); os.remove(p + ".json")
shutil.rmtree(SRC, ignore_errors=True); os.makedirs(SRC)
for v in order:
    d = f"{X}/{v}"; shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    subprocess.run(f"cd {d} && nwn_erf -x -f {S}/home/modules/sp{v}.mod >/dev/null", shell=True, check=True)
    for fn in os.listdir(d):
        if fn.startswith(f"sp{v}.") or (v == order[0] and not fn.startswith("sp")): shutil.copy(f"{d}/{fn}", SRC)
    a = gff(f"{SRC}/sp{v}.are")
    a["DayNightCycle"] = {"type": "byte", "value": 0}; a["IsNight"] = {"type": "byte", "value": 0}
    put(a, f"{SRC}/sp{v}.are")
m = gff(f"{SRC}/module.ifo")
m["Mod_Area_list"]["value"] = [{"__struct_id": 6, "Area_Name": {"type": "resref", "value": f"sp{v}"}} for v in order]
m["Mod_HakList"]["value"] = [{"__struct_id": 8, "Mod_Hak": {"type": "cexostring", "value": h}} for h in [f"sp{v}" for v in order] + ["ls_textures"]]
for k in ("Mod_OnClientEntr", "Mod_OnModLoad"): m[k] = {"type": "resref", "value": "sp_tour"}
m["Mod_Entry_Area"]["value"] = f"sp{order[0]}"
vt = m.setdefault("VarTable", {"type": "list", "value": []})
vt["value"] = [{"__struct_id": 0, "Name": {"type": "cexostring", "value": "SP_ORDER"}, "Type": {"type": "dword", "value": 3}, "Value": {"type": "cexostring", "value": order}}]
put(m, f"{SRC}/module.ifo")
subprocess.run(f"nwn_script_comp -d {SRC} -c {S}/shoot/nss/sp_tour.nss", shell=True, check=True)
o = f"{S}/shoot/sp_shoot.mod"
if os.path.exists(o): os.remove(o)
subprocess.run(f"cd {SRC} && nwn_erf -c -f {o} * >/dev/null", shell=True, check=True); print(o, os.path.getsize(o), os.listdir(SRC))
