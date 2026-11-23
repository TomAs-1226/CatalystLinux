"""Build the print-ready Creality Print project plates (.3mf) for the Catalyst case.

usage: py -3 -I make_plates.py            build the plates, then slice each one to check it
       py -3 -I make_plates.py --no-slice  build only

Printer: Creality SPARKX i7, 0.4 mm nozzle, 260 x 260 mm bed, CFS Lite.
Settings: Creality Print's own SPARKX i7 system profiles, resolved through their inheritance
chains, plus the overrides below (from ../PRINTING.md).

Needs, under .tools/ (gitignored):
  dl-crealityprint/x/  Creality Print 7.3.2 (crealityprint-7.3.2-release-full.nupkg, unzipped):
                       its profiles, and its CLI for the check slice
  dl-orca/x/           OrcaSlicer 2.5.2 portable: writes the geometry and the plate thumbnails
                       (Creality Print's CLI cannot export a 3MF)
"""
import glob, json, os, re, shutil, struct, subprocess, sys, tempfile, zipfile
from datetime import date

HERE = os.path.dirname(os.path.abspath(__file__))
STL = os.path.join(os.path.dirname(HERE), 'stl')
TOOLS = os.path.join(HERE, '.tools')
CP_DIR = os.path.join(TOOLS, 'dl-crealityprint', 'x', 'lib', 'net45')
CP_EXE = os.path.join(CP_DIR, 'CrealityPrint.exe')
CP_PROFILES = os.path.join(CP_DIR, 'resources', 'profiles', 'Creality')
ORCA_EXE = os.path.join(TOOLS, 'dl-orca', 'x', 'orca-slicer.exe')
WORK = os.path.join(TOOLS, 'work')
CP_VERSION = '7.3.2.6412'

PRINTER = 'SPARKX i7 0.4 nozzle'
BED = 260.0
BED_TYPE = 'Textured PEI Plate'   # the i7's stock plate
PLATE_GAP = 1 / 5                 # Creality Print's LOGICAL_PART_PLATE_GAP

# ---- settings (PRINTING.md) -------------------------------------------------------------
# Global process overrides: appearance first (slower outer wall and top skin, seam at the
# rear), 4 walls, gyroid, no supports, brims only where a part asks for one.
PROCESS = {
    'seam_position': 'back',
    'wall_loops': '4',
    'sparse_infill_pattern': 'gyroid',
    'bottom_shell_layers': '4',
    'outer_wall_speed': '120',
    'top_surface_speed': '120',
    'bridge_speed': '40',          # the frame's 43 mm side-window bridges
    'enable_support': '0',
    'brim_type': 'no_brim',
    'skirt_loops': '0',
}
SLOW_FIRST_LAYER = {'initial_layer_speed': '30', 'initial_layer_infill_speed': '45'}

PLA = {'nozzle_temperature': '215', 'nozzle_temperature_initial_layer': '220',
       'textured_plate_temp': '60', 'textured_plate_temp_initial_layer': '60',
       'hot_plate_temp': '60', 'hot_plate_temp_initial_layer': '60'}
PETG = {'nozzle_temperature': '240', 'nozzle_temperature_initial_layer': '240',
        'textured_plate_temp': '80', 'textured_plate_temp_initial_layer': '80',
        'hot_plate_temp': '80', 'hot_plate_temp_initial_layer': '80',
        'fan_min_speed': '30', 'fan_max_speed': '50', 'overhang_fan_speed': '70',
        'close_fan_the_first_x_layers': '3'}
FILAMENTS = {  # key: (Creality Print system preset, colour, overrides)
    'black':      ('Generic PLA @SPARKX i7 0.4 nozzle', '#1B1B1B', PLA),
    'white':      ('Generic PETG @SPARKX i7 0.4 nozzle', '#F2F2F2', PETG),
    'gold':       ('Generic PLA @SPARKX i7 0.4 nozzle', '#C9A227', PLA),
    'light-blue': ('Generic PLA @SPARKX i7 0.4 nozzle', '#8CC8EE', PLA),
}

def part(layer, walls, infill, brim=False):
    s = {'layer_height': layer, 'wall_loops': str(walls), 'sparse_infill_density': infill}
    if infill != '100%':
        s['sparse_infill_pattern'] = 'gyroid'
    if brim:
        s.update(brim_type='outer_only', brim_width='3', brim_object_gap='0.1')
    return s

# stl, object name, per-object settings, centre on the bed (x, y)
PLATES = [
    dict(file='01-black-PLA', name='Black PLA', filament='black',
         process='0.20mm Standard @SPARKX i7 0.4 nozzle', parts=[
             ('frame__black-PLA.stl', 'Frame', part('0.2', 4, '20%'), (130, 200.5)),
             ('back-camera__black-PLA.stl', 'Camera backplate', part('0.2', 4, '20%'), (130, 130)),
             ('back-plain__black-PLA.stl', 'Plain backplate', part('0.2', 4, '20%'), (130, 59.5))]),
    dict(file='02-white-PETG', name='White PETG', filament='white', petg=True,
         process='0.20mm Standard @SPARKX i7 0.4 nozzle', parts=[
             ('chassis__white-PETG.stl', 'Chassis', part('0.2', 4, '40%'), (118, 130)),
             ('plunger-camera__white-PETG.stl', 'Button-B plunger', part('0.12', 4, '100%', True), (183, 130))]),
    dict(file='03-gold-PLA', name='Gold PLA', filament='gold',
         process='0.12mm Fine @SPARKX i7 0.4 nozzle', parts=[
             ('trim-camera__gold-PLA.stl', 'Camera trim', part('0.12', 3, '100%'), (130, 140)),
             ('accent-plain__gold-PLA.stl', 'Gold accent', part('0.12', 3, '100%', True), (130, 112))]),
    dict(file='04-light-blue-PLA', name='Light-blue PLA', filament='light-blue',
         process='0.16mm Standard @SPARKX i7 0.4 nozzle', parts=[
             ('link-cover-camera__light-blue-PLA.stl', 'Link cover', part('0.16', 4, '30%'), (130, 130))]),
]
ALL = dict(file='all-plates', process='0.20mm Standard @SPARKX i7 0.4 nozzle')

# Orca-side project keys Creality Print also reads; everything else comes from Creality's profiles
ORCA_KEEP = ['flush_volumes_matrix', 'flush_volumes_vector', 'flush_multiplier', 'filament_map',
             'filament_map_mode', 'wipe_tower_x', 'wipe_tower_y', 'first_layer_print_sequence',
             'other_layers_print_sequence', 'other_layers_print_sequence_nums', 'filament_self_index',
             'post_process', 'notes', 'printer_notes', 'print_compatible_printers']
META = {'name', 'from', 'inherits', 'instantiation', 'setting_id', 'type', 'compatible_printers',
        'compatible_printers_condition', 'compatible_prints', 'compatible_prints_condition',
        'filament_settings_id', 'print_settings_id', 'printer_settings_id', 'renamed_from',
        'filament_id', 'version', 'description'}

# ---- Creality Print profiles ------------------------------------------------------------
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

def scalar(v):
    return v[0] if isinstance(v, list) else v

def project_config(proc_name, proc_over, fil_keys, orca_cfg):
    machine, process = profile('machine', PRINTER), profile('process', proc_name)
    fils = [profile('filament', FILAMENTS[k][0]) for k in fil_keys]
    cfg = {k: orca_cfg[k] for k in ORCA_KEEP if k in orca_cfg}
    for src in (machine, process):
        cfg.update({k: v for k, v in src.items() if k not in META})
    cfg.update(proc_over)
    fkeys = sorted({k for f in fils for k in f} - META)
    for k in fkeys:
        cfg[k] = [str(scalar(FILAMENTS[key][2].get(k, f.get(k, fils[0].get(k, '')))))
                  for key, f in zip(fil_keys, fils)]
    cfg.update({
        'name': 'project_settings', 'from': 'project', 'version': CP_VERSION,
        'printer_settings_id': PRINTER, 'print_settings_id': proc_name,
        'filament_settings_id': [FILAMENTS[k][0] for k in fil_keys],
        'filament_ids': [str(scalar(f.get('filament_id', ''))) for f in fils],
        'filament_colour': [FILAMENTS[k][1] for k in fil_keys],
        'curr_bed_type': BED_TYPE,
        'inherits_group': [''] * (len(fil_keys) + 2),
        'different_settings_to_system': [';'.join(sorted(proc_over))]
            + [';'.join(sorted(FILAMENTS[k][2])) for k in fil_keys] + [''],
    })
    return dict(sorted(cfg.items()))

# ---- geometry ---------------------------------------------------------------------------
def placed_stl(src, centre, dst):
    """Copy a binary STL, moved so its footprint is centred on `centre` and it sits on z=0."""
    d = open(src, 'rb').read()
    n = struct.unpack_from('<I', d, 80)[0]
    assert 84 + 50 * n == len(d), f'{src}: not a binary STL'
    v = [struct.unpack_from('<12f', d, 84 + 50 * i) for i in range(n)]
    xs = [t[j] for t in v for j in (3, 6, 9)]
    ys = [t[j] for t in v for j in (4, 7, 10)]
    zs = [t[j] for t in v for j in (5, 8, 11)]
    dx = centre[0] - (min(xs) + max(xs)) / 2
    dy = centre[1] - (min(ys) + max(ys)) / 2
    dz = -min(zs)
    out = bytearray(d[:84])
    for i, t in enumerate(v):
        t = list(t)
        for j in (3, 6, 9):
            t[j] += dx; t[j + 1] += dy; t[j + 2] += dz
        out += struct.pack('<12f', *t) + d[84 + 50 * i + 48: 84 + 50 * i + 50]
    open(dst, 'wb').write(out)

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

def orca_export(tag, plate, fil_keys):
    """Orca writes the plate's geometry, object entries and thumbnails."""
    d = os.path.join(WORK, tag)
    shutil.rmtree(d, ignore_errors=True); os.makedirs(d)
    m = flat_json('machine', PRINTER, {}, os.path.join(d, 'machine.json'))
    p = flat_json('process', plate['process'], {}, os.path.join(d, 'process.json'))
    fs = [flat_json('filament', FILAMENTS[k][0], {'filament_colour': [FILAMENTS[k][1]]}, os.path.join(d, f'fil{i}.json'))
          for i, k in enumerate(fil_keys)]
    stls = []
    for stl, _, _, centre in plate.get('parts', []):
        stls.append(os.path.join(d, stl)); placed_stl(os.path.join(STL, stl), centre, stls[-1])
    out = os.path.join(d, 'orca.3mf')
    rc = run([ORCA_EXE, '--datadir', os.path.join(TOOLS, 'orca-data'),
              '--load-settings', f'{m};{p}', '--load-filaments', ';'.join(fs),
              '--arrange', '0', '--orient', '0', '--export-3mf', out] + stls,
             os.path.join(d, 'orca.log'))
    assert rc == 0 and os.path.exists(out), f'Orca export failed for {tag}: see {d}/orca.log'
    return zipfile.ZipFile(out)

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
    return model

def object_settings(obj_xml, name, settings, extruder):
    obj_xml = re.sub(r'(<object id="\d+"[^>]*>\s*)<metadata key="name" value="[^"]*"/>',
                     lambda m: m.group(1) + f'<metadata key="name" value="{esc(name)}"/>', obj_xml, count=1)
    obj_xml = re.sub(r'<metadata key="extruder" value="\d+"/>',
                     f'<metadata key="extruder" value="{extruder}"/>'
                     + ''.join(f'\n    <metadata key="{k}" value="{v}"/>' for k, v in settings.items()),
                     obj_xml, count=1)
    return obj_xml

def plate_settings(plate_xml, idx, name):
    plate_xml = re.sub(r'<metadata key="plater_id" value="\d+"/>', f'<metadata key="plater_id" value="{idx}"/>', plate_xml)
    plate_xml = re.sub(r'<metadata key="plater_name" value="[^"]*"/>',
                       f'<metadata key="plater_name" value="{esc(name)}"/>', plate_xml)
    return re.sub(r'Metadata/(plate|plate_no_light|top|pick)_\d+\.png', rf'Metadata/\g<1>_{idx}.png', plate_xml)

def write_3mf(path, files):
    tmp = path + '.tmp'
    with zipfile.ZipFile(tmp, 'w', zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(name, data)
    os.replace(tmp, path)

def build_plate(plate):
    z = orca_export(plate['file'], plate, [plate['filament']])
    files = {n: z.read(n) for n in z.namelist()}
    orca_cfg = json.loads(files['Metadata/project_settings.config'])
    over = dict(PROCESS, **(SLOW_FIRST_LAYER if plate.get('petg') else {}))
    files['Metadata/project_settings.config'] = json.dumps(
        project_config(plate['process'], over, [plate['filament']], orca_cfg), indent=4).encode()
    ms = files['Metadata/model_settings.config'].decode()
    for stl, name, settings, _ in plate['parts']:
        ms = re.sub(rf'<object id="\d+"[^>]*>\s*<metadata key="name" value="{re.escape(stl)}"/>.*?</object>',
                    lambda m: object_settings(m.group(0), name, settings, 1), ms, flags=re.S)
    ms = re.sub(r'<plate>.*?</plate>', lambda m: plate_settings(m.group(0), 1, plate['name']), ms, flags=re.S)
    files['Metadata/model_settings.config'] = ms.encode()
    files['3D/3dmodel.model'] = model_header(files['3D/3dmodel.model'].decode(),
                                             f"Catalyst case - {plate['name']}").encode()
    files['Metadata/creality.config'] = CREALITY_CONFIG.format(v=CP_VERSION, d=date.today().isoformat()).encode()
    out = os.path.join(HERE, plate['file'] + '.3mf')
    write_3mf(out, files)
    return out, files

def build_all(built):
    """Merge the plate files into one project: plate i keeps its layout, moved to plate i's slot."""
    fil_keys = [p['filament'] for p in PLATES]
    cols = 2 if len(PLATES) <= 4 else 3
    stride = BED * (1 + PLATE_GAP)
    z = orca_export('all', dict(process=ALL['process'], parts=PLATES[-1]['parts']), fil_keys)  # 4-filament config base
    orca_cfg = json.loads(z.read('Metadata/project_settings.config'))
    first = built[0]
    files = {n: d for n, d in first.items() if not n.startswith(('3D/', 'Metadata/plate', 'Metadata/top',
                                                                 'Metadata/pick', 'Metadata/model_settings'))}
    objects, components, items, rels, ms_objects, ms_plates = [], [], [], [], [], []
    next_id, next_ident = 1, 1000
    for i, (plate, pf) in enumerate(zip(PLATES, built)):
        ox, oy = (i % cols) * stride, -(i // cols) * stride
        model = pf['3D/3dmodel.model'].decode()
        ms = pf['Metadata/model_settings.config'].decode()
        idmap = {}
        for m in re.finditer(r'<object id="(\d+)"[^>]*>\s*<components>\s*<component p:path="([^"]+)" objectid="(\d+)"[^>]*transform="([^"]+)"/>',
                             model):
            oid, path, sub, tf = m.groups()
            sub_new, oid_new = next_id, next_id + 1
            next_id += 2
            idmap[oid] = str(oid_new)
            sub_xml = pf[path.lstrip('/')].decode()
            sub_xml = re.sub(r'<object id="\d+" p:UUID="[^"]+"',
                             f'<object id="{sub_new}" p:UUID="{sub_new:04x}0000-81cb-4c03-9d28-80fed5dfa1dc"', sub_xml, count=1)
            files[path.lstrip('/')] = sub_xml.encode()
            rels.append(path)
            objects.append(f'  <object id="{oid_new}" p:UUID="{oid_new:08x}-61cb-4c03-9d28-80fed5dfa1dc" type="model">\n'
                           f'   <components>\n    <component p:path="{path}" objectid="{sub_new}" '
                           f'p:UUID="{sub_new:04x}0000-b206-40ff-9872-83e8017abed1" transform="{tf}"/>\n   </components>\n  </object>')
        for m in re.finditer(r'<item objectid="(\d+)"[^>]*transform="([^"]+)"[^>]*/>', model):
            oid, tf = m.groups()
            t = tf.split()
            t[9] = f'{float(t[9]) + ox:.6g}'; t[10] = f'{float(t[10]) + oy:.6g}'
            nid = idmap[oid]
            items.append(f'  <item objectid="{nid}" p:UUID="{int(nid):08x}-b1ec-4553-aec9-835e5b724bb4" '
                         f'transform="{" ".join(t)}" printable="1"/>')
        for m in re.finditer(r'<object id="(\d+)".*?</object>', ms, flags=re.S):
            x = re.sub(r'<object id="\d+"', f'<object id="{idmap[m.group(1)]}"', m.group(0), count=1)
            x = re.sub(r'<part id="\d+"', f'<part id="{int(idmap[m.group(1)]) - 1}"', x)
            x = re.sub(r'<metadata key="extruder" value="\d+"/>', f'<metadata key="extruder" value="{i + 1}"/>', x)
            ms_objects.append('  ' + x)
        pl = re.search(r'<plate>.*?</plate>', ms, flags=re.S).group(0)
        pl = plate_settings(pl, i + 1, plate['name'])
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
    head = model_header(first['3D/3dmodel.model'].decode(), 'Catalyst case - all plates')
    head = head[:head.index('<resources>')]
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
    over = dict(PROCESS, **SLOW_FIRST_LAYER)  # one process for every plate: keep PETG's slow first layer
    files['Metadata/project_settings.config'] = json.dumps(
        project_config(ALL['process'], over, fil_keys, orca_cfg), indent=4).encode()
    seq = {f'plate_{i + 1}': {'nozzle_sequence': [], 'optimal_assignment': [], 'sequence': []} for i in range(len(PLATES))}
    files['Metadata/filament_sequence.json'] = json.dumps(seq).encode()
    out = os.path.join(HERE, ALL['file'] + '.3mf')
    write_3mf(out, files)
    return out

# ---- check slice ------------------------------------------------------------------------
def check_slice(path):
    """Slice with Creality Print's CLI; return [(plate, time, grams)]. The G-code is deleted."""
    out = tempfile.mkdtemp(dir=WORK)
    rc = run([CP_EXE, '--cli', '--datadir', os.path.join(TOOLS, 'cp-data'), '--slice', '0',
              '--need-gcode-file', '--outputdir', out, path], os.path.join(out, 'cp.log'))
    res = []
    for g in sorted(glob.glob(os.path.join(out, 'plate_*.gcode'))):
        head = open(g, encoding='utf-8', errors='replace').read(200000)
        tail = open(g, 'rb').read()[-400000:].decode('utf-8', 'replace')
        t = re.search(r'estimated printing time \(normal mode\) = ([^\n]+)', head + tail)
        w = re.search(r'total filament used \[g\] = ([\d.]+)', head + tail)
        grams = w and w.group(1)
        res.append((os.path.basename(g)[:-6], t and t.group(1).strip(), grams))
    keep = rc != 0 or not res or '--keep-gcode' in sys.argv
    if not keep:
        shutil.rmtree(out)
    return rc, res, out

if __name__ == '__main__':
    os.makedirs(WORK, exist_ok=True)
    built = []
    for plate in PLATES:
        path, files = build_plate(plate)
        built.append(files)
        print('wrote', os.path.relpath(path, HERE))
    print('wrote', os.path.relpath(build_all(built), HERE))
    if '--no-slice' not in sys.argv:
        for f in [p['file'] for p in PLATES] + [ALL['file']]:
            rc, res, out = check_slice(os.path.join(HERE, f + '.3mf'))
            print(f'{f}.3mf: exit {rc}', res, '' if res and rc == 0 else f'(log in {out})')
