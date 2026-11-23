"""Slice every STL in stl/ with PrusaSlicer's CLI and report time and filament.

usage: python tools/estimate.py            (PrusaSlicer: $PRUSASLICER or .tools/prusaslicer)
The profile is a fast CoreXY printer on a 256 x 256 bed with a 0.4 mm nozzle (Bambu P1/X1
class). A bed-slinger at stock speeds takes about twice as long.
"""
import glob, json, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

BASE = """
bed_shape = 0x0,256x0,256x256,0x256
max_print_height = 256
gcode_flavor = marlin2
nozzle_diameter = 0.4
filament_diameter = 1.75
first_layer_height = 0.2
top_solid_layers = 5
bottom_solid_layers = 4
fill_pattern = gyroid
top_fill_pattern = monotonic
bottom_fill_pattern = monotonic
support_material = 0
skirts = 1
skirt_distance = 4
brim_width = 0
perimeter_speed = 150
external_perimeter_speed = 120
small_perimeter_speed = 80
infill_speed = 250
solid_infill_speed = 200
top_solid_infill_speed = 120
bridge_speed = 40
gap_fill_speed = 100
travel_speed = 400
first_layer_speed = 50
default_acceleration = 5000
perimeter_acceleration = 4000
external_perimeter_acceleration = 3000
infill_acceleration = 8000
bridge_acceleration = 2000
first_layer_acceleration = 1000
travel_acceleration = 10000
machine_limits_usage = time_estimate_only
machine_max_acceleration_extruding = 10000
machine_max_acceleration_travel = 10000
machine_max_acceleration_x = 10000
machine_max_acceleration_y = 10000
machine_max_feedrate_x = 500
machine_max_feedrate_y = 500
machine_max_feedrate_z = 20
machine_max_jerk_x = 9
machine_max_jerk_y = 9
"""
MATERIAL = {
    "PLA": "filament_density = 1.24\nfilament_cost = 20\nmax_volumetric_speed = 20\ntemperature = 215\nbed_temperature = 60\n",
    "PETG": "filament_density = 1.27\nfilament_cost = 22\nmax_volumetric_speed = 12\ntemperature = 240\nbed_temperature = 80\n",
}
# per part: layer height, walls, infill
PART = {   # layer height, walls, infill. 5 walls make the 2 mm walls solid.
    "frame": (0.2, 5, "25%"),
    "back-camera": (0.2, 5, "25%"),
    "back-plain": (0.2, 5, "25%"),
    "lens-ring-camera": (0.16, 4, "100%"),
    "link-cover-camera": (0.16, 4, "30%"),
    "pins-camera": (0.12, 4, "100%"),
    "fit-test": (0.2, 2, "15%"),
    "chassis": (0.2, 4, "40%"),            # frozen, already printed
    "plunger-camera": (0.12, 4, "100%"),   # frozen, already printed
}


def slicer():
    if os.environ.get("PRUSASLICER"):
        return os.environ["PRUSASLICER"]
    found = glob.glob(os.path.join(HERE, ".tools", "prusaslicer", "*", "prusa-slicer-console.exe"))
    return found[0] if found else "prusa-slicer-console"


def run(stl):
    name = os.path.basename(stl)[:-4]
    part, mat = name.split("__")
    material = "PETG" if "PETG" in mat else "PLA"
    lh, walls, fill = PART[part]
    ini = BASE + MATERIAL[material] + f"layer_height = {lh}\nperimeters = {walls}\nfill_density = {fill}\n"
    if fill == "100%":
        ini = ini.replace("fill_pattern = gyroid", "fill_pattern = rectilinear")
    with tempfile.TemporaryDirectory() as td:
        cfg, out = os.path.join(td, "p.ini"), os.path.join(td, "p.gcode")
        open(cfg, "w").write(ini)
        r = subprocess.run([slicer(), "--export-gcode", "--load", cfg, "--center", "128,128",
                            "--output", out, stl], capture_output=True, text=True)
        if r.returncode != 0 or not os.path.exists(out):
            return dict(part=part, error=(r.stderr or r.stdout)[-400:])
        tail = open(out, errors="ignore").read()[-30000:]
    t = re.search(r"estimated printing time \(normal mode\) = (.+)", tail)
    g = re.search(r"filament used \[g\] = ([\d.]+)", tail)
    m = re.search(r"filament used \[mm\] = ([\d.]+)", tail)
    return dict(part=part, material=mat, layer=lh, walls=walls, infill=fill,
                time=t.group(1).strip() if t else "?", grams=float(g.group(1)) if g else 0.0,
                metres=round(float(m.group(1)) / 1000, 2) if m else 0.0)


def minutes(s):
    d = dict(re.findall(r"(\d+)([dhms])", s))
    return int(d.get("d", 0)) * 1440 + int(d.get("h", 0)) * 60 + int(d.get("m", 0)) + int(d.get("s", 0)) / 60


def main():
    rows = [run(f) for f in sorted(glob.glob(os.path.join(HERE, "stl", "*.stl")))]
    for r in rows:
        if "error" in r:
            print(r["part"], "ERROR", r["error"])
        else:
            print(f"{r['part']:20s} {r['material']:16s} {r['layer']:.2f} mm {r['walls']} walls {r['infill']:>4s}  "
                  f"{r['time']:>10s}  {r['grams']:6.1f} g  {r['metres']:5.2f} m")
            r["minutes"] = round(minutes(r["time"]), 1)
    os.makedirs(os.path.join(HERE, "out"), exist_ok=True)
    json.dump(rows, open(os.path.join(HERE, "out", "estimates.json"), "w"), indent=1)


if __name__ == "__main__":
    main()
