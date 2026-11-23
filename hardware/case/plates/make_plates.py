"""Build the print-ready Creality Print project for the Catalyst case: every part in black PLA
on a Creality SPARKX i7, packed onto as few plates as fit.

usage: py -3 -I make_plates.py               write black-PLA.3mf, then check-slice it
       py -3 -I make_plates.py --no-slice    write only
       py -3 -I make_plates.py --keep-gcode  keep the check slice's G-code in .tools/work/

Printer: SPARKX i7 0.4 nozzle (direct drive), 260 x 260 mm textured PEI plate, CFS Lite.
Settings: Creality Print's own SPARKX i7 system presets (machine, "0.20mm Standard", "Generic
PLA"), resolved through their inheritance chains, plus the black-PLA profile below. Per-part
layer height, walls, infill, brim and support placement go on each object.

Every STL in ../stl/ is printed except those in SKIP, in the orientation it was exported in.
Parts the tables below do not name get DEFAULT_PART.

Needs, under .tools/ (gitignored), and numpy:
  dl-crealityprint/x/  Creality Print 7.3.2 (crealityprint-7.3.2-release-full.nupkg, unzipped):
                       its profiles, and its CLI for the check slice
  dl-orca/x/           OrcaSlicer 2.5.2 portable: writes the geometry and the plate thumbnails
                       (Creality Print's CLI cannot export a 3MF)
"""
import glob, json, os, re, shutil, struct, subprocess, sys, tempfile, zipfile
from datetime import date

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
STL = os.path.join(os.path.dirname(HERE), 'stl')
TOOLS = os.path.join(HERE, '.tools')
CP_DIR = os.path.join(TOOLS, 'dl-crealityprint', 'x', 'lib', 'net45')
CP_EXE = os.path.join(CP_DIR, 'CrealityPrint.exe')
CP_PROFILES = os.path.join(CP_DIR, 'resources', 'profiles', 'Creality')
ORCA_EXE = os.path.join(TOOLS, 'dl-orca', 'x', 'orca-slicer.exe')
WORK = os.path.join(TOOLS, 'work')
CP_VERSION = '7.3.2.6412'
OUT = 'black-PLA'

PRINTER = 'SPARKX i7 0.4 nozzle'
PROCESS_PRESET = '0.20mm Standard @SPARKX i7 0.4 nozzle'
FILAMENT_PRESET = 'Generic PLA @SPARKX i7 0.4 nozzle'
COLOUR = '#1B1B1B'
BED = 260.0
BED_TYPE = 'Textured PEI Plate'   # the i7's stock plate
PLATE_GAP = 1 / 5                 # Creality Print's LOGICAL_PART_PLATE_GAP
EDGE = 6.0                        # keep brims this far from the bed edge
CLEAR = 10.0                      # clear space between neighbouring brims

# ---- the black-PLA profile ---------------------------------------------------------------
BRIM, SMALL_BRIM = 7, 5
SUPPORT_ANGLE = 40                # support faces flatter than this (degrees from horizontal)

PROCESS = {
    # adhesion: thick, slow, slightly wide first layer, and a brim on every part
    'initial_layer_print_height': '0.25',
    'initial_layer_speed': '30',
    'initial_layer_infill_speed': '35',
    'initial_layer_line_width': '0.44',          # 110 % of the nozzle
    'brim_type': 'outer_only',
    'brim_width': str(BRIM),
    'brim_object_gap': '0.1',
    'skirt_loops': '0',
    # supports: off. A part that truly needs them turns them on in PARTS and gets organic
    # tree supports from the build plate, with interface layers so surfaces come off clean.
    'enable_support': '0',
    'support_type': 'tree(auto)',
    'support_style': 'organic',
    'support_threshold_angle': str(SUPPORT_ANGLE),
    'support_on_build_plate_only': '1',
    'support_interface_top_layers': '3',
    'support_interface_bottom_layers': '2',
    'support_top_z_distance': '0.2',
    'support_bottom_z_distance': '0.2',
    # stringing: no wall crossing on travel (wipe and retraction are on the filament)
    'reduce_crossing_wall': '1',
    'travel_speed': '400',
    # speed: calm outer walls, slower still on overhangs and bridges
    'outer_wall_speed': '70',
    'inner_wall_speed': '150',
    'top_surface_speed': '100',
    'enable_overhang_speed': '1',
    'overhang_1_4_speed': '50',
    'overhang_2_4_speed': '30',
    'overhang_3_4_speed': '20',
    'overhang_4_4_speed': '10',
    'bridge_speed': '30',
    # look and strength
    'seam_position': 'back',
    'wall_loops': '5',
    'sparse_infill_pattern': 'gyroid',
    'bottom_shell_layers': '4',
}
FILAMENT = {
    # cool against stringing (210 C sits in both PRINTING.md's range and the anti-stringing
    # one), a little hotter for the first layer; flow capped to what 210 C melts cleanly
    'nozzle_temperature': '210',
    'nozzle_temperature_initial_layer': '215',
    'filament_max_volumetric_speed': '11',
    'textured_plate_temp': '60', 'textured_plate_temp_initial_layer': '60',
    'hot_plate_temp': '60', 'hot_plate_temp_initial_layer': '60',
    'cool_plate_temp': '60', 'cool_plate_temp_initial_layer': '60',
    # direct-drive retraction: the preset's 0.8 mm @ 30 mm/s, a little longer and quicker
    'filament_retraction_length': '1.0',
    'filament_retraction_speed': '40',
    'filament_deretraction_speed': '30',
    'filament_wipe': '1',
    'filament_wipe_distance': '2',
    'filament_z_hop': '0.2',
    # no fan on the first two layers, full from layer 4
    'close_fan_the_first_x_layers': '2',
    'full_fan_speed_layer': '4',
    'fan_max_speed': '100',
}

def part(name, layer='0.2', walls=5, infill='25%', brim=BRIM, support=None):
    """support: None (off), 'plate' (from the build plate only) or 'everywhere'."""
    s = {'layer_height': layer, 'wall_loops': str(walls), 'sparse_infill_density': infill}
    if infill != '100%':
        s['sparse_infill_pattern'] = 'gyroid'
    if brim != BRIM:
        s['brim_width'] = str(brim)
    if support:
        s['enable_support'] = '1'
        s['support_on_build_plate_only'] = '1' if support == 'plate' else '0'
    return name, s

# per part, by STL name before "__" (PRINTING.md, revision 3). The script prints each part's
# overhang area; re-check these after a CAD change. Revision 3 needs no supports: the only
# overhangs are 0.4 mm grip-recess ceilings on the backplates' bed side (bridged), the
# bezel's shallow inner edge in its first millimetre, and slivers elsewhere.
PARTS = {
    'frame': part('Bezel'),
    'back-camera': part('Camera backplate'),
    'back-plain': part('Plain backplate'),
    'lens-ring-camera': part('Lens ring', '0.16', 4, '100%', SMALL_BRIM),
    'link-cover-camera': part('Link cover', '0.16', 4, '30%', SMALL_BRIM),
    'pins-camera': part('UnitV pins', '0.12', 4, '100%', SMALL_BRIM),
}
# not on the black plate: the fit test, and the chassis and plunger (printed in white PETG;
# their geometry does not change)
SKIP = ('fit-test', 'chassis', 'plunger-camera')

def default_part(key):
    return part(key.replace('-', ' ').capitalize())

# Orca-side project keys Creality Print also reads; everything else comes from Creality's presets
ORCA_KEEP = ['flush_volumes_matrix', 'flush_volumes_vector', 'flush_multiplier', 'filament_map',
             'filament_map_mode', 'wipe_tower_x', 'wipe_tower_y', 'first_layer_print_sequence',
             'other_layers_print_sequence', 'other_layers_print_sequence_nums', 'filament_self_index',
             'post_process', 'notes', 'printer_notes', 'print_compatible_printers']
META = {'name', 'from', 'inherits', 'instantiation', 'setting_id', 'type', 'compatible_printers',
        'compatible_printers_condition', 'compatible_prints', 'compatible_prints_condition',
        'filament_settings_id', 'print_settings_id', 'printer_settings_id', 'renamed_from',
        'filament_id', 'version', 'description'}

# ---- Creality Print presets --------------------------------------------------------------
_index = {}
def profile(cat, name):
    """A Creality Print system preset with its inheritance chain resolved."""
    if not _index:
        for c in ('machine', 'process', 'filament'):
            for f in glob.glob(os.path.join(CP_PROFILES, c, '*.json')):
                with open(f, encoding='utf-8') as fh:
                    j = json.load(fh)
                if 'name' in j:
                    _index[(c, j['name'])] = j
    j = _index[(cat, name)]
    out = profile(cat, j['inherits']) if j.get('inherits') else {}
    out.update(j)
    return out

def project_config(orca_cfg):
    machine, process = profile('machine', PRINTER), profile('process', PROCESS_PRESET)
    fil = profile('filament', FILAMENT_PRESET)
    cfg = {k: orca_cfg[k] for k in ORCA_KEEP if k in orca_cfg}
    for src in (machine, process):
        cfg.update({k: v for k, v in src.items() if k not in META})
    cfg.update(PROCESS)
    for k in set(fil) - META:
        v = FILAMENT.get(k, fil[k])
        cfg[k] = [str(v[0] if isinstance(v, list) else v)]
    for k, v in FILAMENT.items():
        cfg[k] = [v]
    cfg.update({
        'name': 'project_settings', 'from': 'project', 'version': CP_VERSION,
        'printer_settings_id': PRINTER, 'print_settings_id': PROCESS_PRESET,
        'filament_settings_id': [FILAMENT_PRESET],
        'filament_ids': [str(fil.get('filament_id', ''))],
        'filament_colour': [COLOUR],
        'curr_bed_type': BED_TYPE,
        'inherits_group': ['', '', ''],
        'different_settings_to_system': [';'.join(sorted(PROCESS)), ';'.join(sorted(FILAMENT)), ''],
    })
    return dict(sorted(cfg.items()))

# ---- geometry ---------------------------------------------------------------------------
def load_stl(path):
    d = open(path, 'rb').read()
    n = struct.unpack_from('<I', d, 80)[0]
    assert 84 + 50 * n == len(d), f'{path}: not a binary STL'
    rec = np.frombuffer(d, dtype=np.dtype([('n', '<3f4'), ('v', '<9f4'), ('a', '<u2')]), count=n, offset=84)
    return d, rec['v'].reshape(-1, 3, 3).astype(np.float64)

def overhangs(T):
    """Overhang area (mm2) flatter than SUPPORT_ANGLE above the first layer: (over the bed,
    over the part), by a ray straight down from each face. For the report only."""
    T = T - [0, 0, T[:, :, 2].min()]
    e1, e2 = T[:, 1] - T[:, 0], T[:, 2] - T[:, 0]
    c = np.cross(e1, e2)
    a2 = np.linalg.norm(c, axis=1)
    nz = c[:, 2] / np.maximum(a2, 1e-12)
    slope = np.degrees(np.arccos(np.clip(-nz, -1, 1)))
    over = (nz < 0) & (slope < SUPPORT_ANGLE) & (T[:, :, 2].min(1) > 0.3) & (a2 > 1e-3)
    d = np.array([0, 0, -1.0])
    pv = np.cross(d, e2)
    det = (e1 * pv).sum(1)
    ok = np.abs(det) > 1e-12
    inv = np.where(ok, 1 / np.where(ok, det, 1), 0)
    bed = part = 0.0
    for o, area in zip(T[over].mean(1), a2[over] / 2):
        tv = o - T[:, 0] - [0, 0, 1e-3]
        u = (tv * pv).sum(1) * inv
        q = np.cross(tv, e1)
        v = (q * d).sum(1) * inv
        t = (q * e2).sum(1) * inv
        if (ok & (u >= 0) & (v >= 0) & (u + v <= 1) & (t > 0.05)).any():
            part += area
        else:
            bed += area
    return bed, part

def placed_stl(raw, T, centre, dst):
    """Write a binary STL moved so its footprint is centred on `centre` and it sits on z=0."""
    lo, hi = T.reshape(-1, 3).min(0), T.reshape(-1, 3).max(0)
    shift = np.array([centre[0] - (lo[0] + hi[0]) / 2, centre[1] - (lo[1] + hi[1]) / 2, -lo[2]])
    rec = np.frombuffer(raw, dtype=np.dtype([('n', '<3f4'), ('v', '<9f4'), ('a', '<u2')]),
                        count=len(T), offset=84).copy()
    rec['v'] = (T + shift).reshape(-1, 9).astype('<f4')
    open(dst, 'wb').write(raw[:84] + rec.tobytes())

def collect_parts():
    parts = []
    for path in sorted(glob.glob(os.path.join(STL, '*.stl'))):
        stl = os.path.basename(path)
        key = stl.split('__')[0]
        if key.startswith(SKIP):
            continue
        name, settings = PARTS.get(key) or default_part(key)
        raw, T = load_stl(path)
        size = T.reshape(-1, 3).max(0) - T.reshape(-1, 3).min(0)
        brim = float(settings.get('brim_width', BRIM))
        parts.append(dict(stl=stl, name=name, settings=settings, raw=raw, T=T, size=size, brim=brim,
                          overhangs=overhangs(T)))
    return parts

def pack(parts):
    """Shelf-pack the parts (with their brims and CLEAR between them) onto as few plates as fit."""
    usable = BED - 2 * EDGE
    cell = lambda p: (p['size'][0] + 2 * p['brim'] + CLEAR, p['size'][1] + 2 * p['brim'] + CLEAR)
    plates, rows, row, x, y, h = [], [], [], 0.0, 0.0, 0.0
    for p in sorted(parts, key=lambda p: (-p['size'][1], -p['size'][0])):
        w, d = cell(p)
        assert w - CLEAR <= usable and d - CLEAR <= usable, f"{p['stl']} does not fit the bed"
        if row and x + w - CLEAR > usable:
            rows.append((row, x, h)); y += h; row, x, h = [], 0.0, 0.0
        if y + d - CLEAR > usable:
            plates.append(rows); rows, y = [], 0.0
        row.append((p, x, w, d)); x += w; h = max(h, d)
    rows.append((row, x, h)); plates.append(rows)
    out = []
    for rows in plates:                          # centre every row, and the rows on the bed
        total = sum(h for _, _, h in rows) - CLEAR
        y = (BED + total) / 2
        placed = []
        for row, width, h in rows:
            x0 = (BED - (width - CLEAR)) / 2
            for p, x, w, d in row:
                placed.append((p, (x0 + x + (w - CLEAR) / 2, y - (d - CLEAR) / 2)))
            y -= h
        out.append(placed)
    return out

def flat_json(cat, name, extra, path):
    p = profile(cat, name)
    p.pop('inherits', None)
    p.update(extra)
    # Orca's range checks reject two Creality-only values; this copy is only for Orca's export
    if cat == 'machine':
        p['retraction_distances_when_cut'] = '18'
    if cat == 'process':
        p['timelapse_type'] = '0'
    with open(path, 'w', encoding='utf-8', newline='\n') as fh:
        json.dump(p, fh, indent=1)
    return path

def run(cmd, log):
    tmp = os.path.join(WORK, 'temp')  # keep the slicers' scratch files out of the user's %TEMP%
    os.makedirs(tmp, exist_ok=True)
    env = dict(os.environ, TEMP=tmp, TMP=tmp)
    with open(log, 'w') as fh:
        r = subprocess.run(cmd, stdout=fh, stderr=subprocess.STDOUT, cwd=WORK, env=env)
    return r.returncode

def orca_export(tag, placed):
    """Orca writes one plate's geometry, object entries and thumbnails."""
    d = os.path.join(WORK, tag)
    shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    m = flat_json('machine', PRINTER, {}, os.path.join(d, 'machine.json'))
    p = flat_json('process', PROCESS_PRESET, {}, os.path.join(d, 'process.json'))
    f = flat_json('filament', FILAMENT_PRESET, {'filament_colour': [COLOUR]}, os.path.join(d, 'fil.json'))
    stls = []
    for part, centre in placed:
        stls.append(os.path.join(d, part['stl'])); placed_stl(part['raw'], part['T'], centre, stls[-1])
    out = os.path.join(d, 'orca.3mf')
    rc = run([ORCA_EXE, '--datadir', os.path.join(TOOLS, 'orca-data'),
              '--load-settings', f'{m};{p}', '--load-filaments', f,
              '--arrange', '0', '--orient', '0', '--export-3mf', out] + stls,
             os.path.join(d, 'orca.log'))
    assert rc == 0 and os.path.exists(out), f'Orca export failed for {tag}: see {d}/orca.log'
    with zipfile.ZipFile(out) as z:
        return {n: z.read(n) for n in z.namelist()}

# ---- 3MF writing ------------------------------------------------------------------------
CREALITY_CONFIG = """<?xml version="1.0" encoding="UTF-8"?>
<config>
    <metadata key="Company" value="Creality"/>
    <metadata key="Application" value="Creality_Print"/>
    <metadata key="AppVersion" value="{v}"/>
    <metadata key="AppStage" value="Release"/>
    <metadata key="FileVersion" value="1.0"/>
    <metadata key="FileType" value="Undefined"/>
    <metadata key="ProfileVersion" value=""/>
    <metadata key="CreationDate" value="{d}"/>
</config>
"""

def esc(s):
    return s.replace('&', '&amp;').replace('"', '&quot;').replace('<', '&lt;')

def model_header(model, title):
    model = re.sub(r'<metadata name="Application">[^<]*</metadata>',
                   f'<metadata name="Application">Creality_Print V{CP_VERSION} Release</metadata>', model)
    model = re.sub(r'<metadata name="OrcaSlicer">[^<]*</metadata>\s*', '', model)
    model = re.sub(r'<metadata name="Title">[^<]*</metadata>', f'<metadata name="Title">{esc(title)}</metadata>', model)
    today = date.today().isoformat()
    for tag in ('CreationDate', 'ModificationDate'):
        model = re.sub(rf'<metadata name="{tag}">[^<]*</metadata>', f'<metadata name="{tag}">{today}</metadata>', model)
    return model[:model.index('<resources>')]

def build_project(plates):
    """One project, one plate per entry of `plates`; plate i sits in Creality Print's slot i."""
    cols = int(np.ceil(np.sqrt(len(plates))))
    stride = BED * (1 + PLATE_GAP)
    files, objects, items, rels, ms_objects, ms_plates = {}, [], [], [], [], []
    next_id, next_ident = 1, 1000
    for i, placed in enumerate(plates):
        pf = orca_export(f'plate{i + 1}', placed)
        if i == 0:
            files.update({n: d for n, d in pf.items() if not n.startswith(('3D/', 'Metadata/plate', 'Metadata/top',
                                                                           'Metadata/pick', 'Metadata/model_settings'))})
            head = model_header(pf['3D/3dmodel.model'].decode(), 'Catalyst case - black PLA')
            cfg = project_config(json.loads(pf['Metadata/project_settings.config']))
        ox, oy = (i % cols) * stride, -(i // cols) * stride
        model = pf['3D/3dmodel.model'].decode()
        ms = pf['Metadata/model_settings.config'].decode()
        by_stl = {part['stl']: part for part, _ in placed}
        idmap = {}
        for m in re.finditer(r'<object id="(\d+)"[^>]*>\s*<components>\s*<component p:path="([^"]+)" objectid="\d+"[^>]*transform="([^"]+)"/>', model):
            oid, path, tf = m.groups()
            sub, new = next_id, next_id + 1
            next_id += 2
            idmap[oid] = new
            sub_xml = re.sub(r'<object id="\d+" p:UUID="[^"]+"', f'<object id="{sub}" p:UUID="{sub:04x}0000-81cb-4c03-9d28-80fed5dfa1dc"',
                             pf[path.lstrip('/')].decode(), count=1)
            files[path.lstrip('/')] = sub_xml.encode()
            rels.append(path)
            objects.append(f'  <object id="{new}" p:UUID="{new:08x}-61cb-4c03-9d28-80fed5dfa1dc" type="model">\n'
                           f'   <components>\n    <component p:path="{path}" objectid="{sub}" '
                           f'p:UUID="{sub:04x}0000-b206-40ff-9872-83e8017abed1" transform="{tf}"/>\n   </components>\n  </object>')
        for m in re.finditer(r'<item objectid="(\d+)"[^>]*transform="([^"]+)"[^>]*/>', model):
            t = m.group(2).split()
            t[9], t[10] = f'{float(t[9]) + ox:.6g}', f'{float(t[10]) + oy:.6g}'
            new = idmap[m.group(1)]
            items.append(f'  <item objectid="{new}" p:UUID="{new:08x}-b1ec-4553-aec9-835e5b724bb4" '
                         f'transform="{" ".join(t)}" printable="1"/>')
        for m in re.finditer(r'<object id="(\d+)".*?</object>', ms, flags=re.S):
            x, new = m.group(0), idmap[m.group(1)]
            stl = re.search(r'<metadata key="name" value="([^"]+)"/>', x).group(1)
            part = by_stl[stl]
            x = re.sub(r'<object id="\d+"', f'<object id="{new}"', x, count=1)
            x = re.sub(r'<part id="\d+"', f'<part id="{new - 1}"', x)
            x = x.replace(f'<metadata key="name" value="{stl}"/>', f'<metadata key="name" value="{esc(part["name"])}"/>', 1)
            x = x.replace('<metadata key="extruder" value="1"/>', '<metadata key="extruder" value="1"/>'
                          + ''.join(f'\n    <metadata key="{k}" value="{v}"/>' for k, v in part['settings'].items()), 1)
            ms_objects.append('  ' + x)
        pl = re.search(r'<plate>.*?</plate>', ms, flags=re.S).group(0)
        pl = re.sub(r'<metadata key="plater_id" value="\d+"/>', f'<metadata key="plater_id" value="{i + 1}"/>', pl)
        pl = re.sub(r'<metadata key="plater_name" value="[^"]*"/>', f'<metadata key="plater_name" value="Black PLA {i + 1}"/>', pl)
        pl = re.sub(r'Metadata/(plate|plate_no_light|top|pick)_\d+\.png', rf'Metadata/\g<1>_{i + 1}.png', pl)
        def inst(m):
            nonlocal next_ident
            next_ident += 1
            return (f'<metadata key="object_id" value="{idmap[m.group(1)]}"/>\n      <metadata key="instance_id" value="0"/>\n'
                    f'      <metadata key="identify_id" value="{next_ident}"/>')
        pl = re.sub(r'<metadata key="object_id" value="(\d+)"/>\s*<metadata key="instance_id" value="0"/>\s*'
                    r'<metadata key="identify_id" value="\d+"/>', inst, pl)
        ms_plates.append('  ' + pl)
        for n, d in pf.items():
            m = re.fullmatch(r'Metadata/(plate|plate_no_light|top|pick)_1(_small)?\.png', n)
            if m:
                files[f'Metadata/{m.group(1)}_{i + 1}{m.group(2) or ""}.png'] = d
    files['3D/3dmodel.model'] = (head + '<resources>\n' + '\n'.join(objects) + '\n </resources>\n'
                                 + ' <build p:UUID="2c7c17d8-22b5-4d84-8835-1976022ea369">\n' + '\n'.join(items)
                                 + '\n </build>\n</model>\n').encode()
    files['3D/_rels/3dmodel.model.rels'] = (
        '<?xml version="1.0" encoding="UTF-8"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">\n'
        + ''.join(f' <Relationship Target="{p}" Id="rel-{k + 1}" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>\n'
                  for k, p in enumerate(rels)) + '</Relationships>').encode()
    files['Metadata/model_settings.config'] = ('<?xml version="1.0" encoding="UTF-8"?>\n<config>\n'
                                               + '\n'.join(ms_objects) + '\n' + '\n'.join(ms_plates)
                                               + '\n  <assemble>\n  </assemble>\n</config>\n').encode()
    files['Metadata/project_settings.config'] = json.dumps(cfg, indent=4).encode()
    files['Metadata/filament_sequence.json'] = json.dumps(
        {f'plate_{i + 1}': {'nozzle_sequence': [], 'optimal_assignment': [], 'sequence': []} for i in range(len(plates))}).encode()
    files['Metadata/creality.config'] = CREALITY_CONFIG.format(v=CP_VERSION, d=date.today().isoformat()).encode()
    out = os.path.join(HERE, OUT + '.3mf')
    with zipfile.ZipFile(out + '.tmp', 'w', zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(name, data)
    os.replace(out + '.tmp', out)
    return out

# ---- check slice ------------------------------------------------------------------------
def check_slice(path):
    """Slice with Creality Print's CLI; return [(plate, time, grams)]. The G-code is deleted."""
    out = tempfile.mkdtemp(dir=WORK)
    rc = run([CP_EXE, '--cli', '--datadir', os.path.join(TOOLS, 'cp-data'), '--slice', '0',
              '--need-gcode-file', '--outputdir', out, path], os.path.join(out, 'cp.log'))
    res = []
    for g in sorted(glob.glob(os.path.join(out, 'plate_*.gcode'))):
        text = open(g, 'rb').read()[-400000:].decode('utf-8', 'replace')
        t = re.search(r'estimated printing time \(normal mode\) = ([^\n]+)', text)
        w = re.search(r'total filament used \[g\] = ([\d.]+)', text)
        res.append((os.path.basename(g)[:-6], t and t.group(1).strip(), w and w.group(1)))
    if rc == 0 and res and '--keep-gcode' not in sys.argv:
        shutil.rmtree(out)
    return rc, res, out

if __name__ == '__main__':
    os.makedirs(WORK, exist_ok=True)
    plates = pack(collect_parts())
    for i, placed in enumerate(plates):
        print(f'plate {i + 1}:')
        for p, _ in placed:
            sup = p['settings'].get('enable_support') == '1'
            sup = ('supports from plate' if p['settings']['support_on_build_plate_only'] == '1'
                   else 'supports everywhere') if sup else 'no supports'
            print(f"  {p['name']}: {sup}; overhangs {p['overhangs'][0]:.0f} mm2 over the bed, "
                  f"{p['overhangs'][1]:.0f} mm2 over the part")
    path = build_project(plates)
    print('wrote', os.path.relpath(path, HERE))
    if '--no-slice' not in sys.argv:
        rc, res, out = check_slice(path)
        print(f'check slice: exit {rc}', res, '' if res and rc == 0 else f'(log in {out})')
