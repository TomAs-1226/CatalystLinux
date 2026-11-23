# Printing and assembling the Catalyst case (revision 3.1)

One **bezel**, one **chassis** and either of two **backplates**.

## What changed after the first print

The first print failed. Revision 3 fixes each cause, and revision 3.1 fixes the thin tray rim found in the slicer:

| Problem | Cause | Fix |
| --- | --- | --- |
| Frame turned to spaghetti | It stood 25.5 mm tall on about 1 mm of lip, and its port windows printed as 12-41 mm bridges | The frame is now a bezel: the 4.9 mm lip and a flat cheek beside the screen, plus a 1.2 mm tongue underneath. It prints face-down on 1,286 mm². The side walls moved into the backplates, and every port window is a notch open at the top of the print on both parts. No bridges, no supports. |
| Back cover came out "weird" | Big shallow pockets on the face that prints on the bed: a 0.5 mm grip panel over 69 × 18 mm and a 0.8 mm trim pocket. Both print as bridges just above the first layers. A 1.5 mm inward ledge inside the lip, 14.6 mm up, needed a ring of supports. The pins were printed upright and weak. | Nothing is recessed on the bed face except bridgeable details: a 0.8 mm groove outlines the grip panel, and the counterbores are 4.2 mm. No inward ledges. The rear edges start with a 45° chamfer. Side walls are 3.3 mm and skins 2 mm, ribs stiffen the pod, and screw towers have 1.7 mm walls. |
| UnitV pins didn't click | Short, and printed upright | Separate click pins, printed lying down |
| Grove plug didn't fit the cable opening | The opening was 3.8 × 8.6 mm | 13 × 9 mm, with a proper channel inside |
| Tray rim was "a thin line" (revision 3) | The trays ended in a 1.0 mm tongue (the inner half of a 2 mm wall) above a 0.3 mm-rounded edge. The slot-hook pocket left 0.8 mm of top wall, and the snap dimples sat 0.7 mm under the rim. | **Revision 3.1.** Side walls are 3.3 mm right up to the rim. The rim is a **flat, square 2.0 mm face**. Its inner 1.3 mm steps down 2.5 mm, and the bezel's **1.2 mm tongue** drops into the step and clicks in with five snaps. Hook pockets now leave 1.7 mm of wall, and the dimples sit 1 mm+ below the rim. `tools/check_thin.py` finds nothing thinner than 1 mm that isn't intentional (section 7). |

**Colour.** Everything new is **black PLA**. The gold trim, gold accent and blue link cover are gone:

- The lens ring and link cover stay separate parts because their shape needs it. The lens ring can still be printed in gold if you like.
- The plain backplate's accent is now a debossed line.

**Frozen parts.** The **chassis and plunger are unchanged**. They are the white PETG parts from the first print, and their STLs are byte-for-byte the same.

| | Camera backplate | Plain backplate |
| --- | --- | --- |
| Thickness | **40.5 mm** (42.9 at the lens ring) | **28.5 mm** |
| Footprint | 91.1 × 61.1 mm (97.1 wide with the link cover) | 91.1 × 61.1 mm |
| New black parts | 57.6 g, about 2 h 04 min | 36.0 g, about 1 h 07 min |
| Weight with the CardputerZero (95 g), chassis, UnitV, cable and screws | about 180 g | about 140 g |

> **Known issue (camera backplate, revision 3.1): do not print it yet.** The
> Grove cable channel comes out as a solid ridge, not a channel:
> `channel2d()`'s `round2d(4.0)` deletes the 5 mm-wide channel, leaving an
> 8.2 mm-wide, 3.5 mm-tall ridge with the cable lips buried in it and only
> 1.3 mm under the UnitV. `check_fit.py` and `check_thin.py` do not catch
> it. The plain backplate is not affected. Fix the model before the camera
> back is printed.

## 1. Parts

All STLs in `stl/` are already oriented for the bed (z = 0 is the bed).

**Profile:** 0.4 mm nozzle, PLA at 210-215 °C, bed at 60 °C. **No supports.**

**Brims:**

- **Bezel:** a 5 mm brim. Its contact is a 4.4 mm ring plus the cheek: 1,286 mm².
- **Backplates:** a 3 mm brim on the corners if your bed lifts corners.

**Estimates:** from `tools/estimate.py` (PrusaSlicer 2.9.2, fast CoreXY profile). A bed-slinger at stock speeds takes about twice as long. Five walls make the 2 mm skins and the walls' 2 mm outer band solid.

| Part | File | Material | **Print face (on the bed)** | Layer | Walls | Infill | Time | Filament |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Bezel (both versions) | `frame__black-PLA.stl` | black PLA | the **front face** (lip and cheek; tongue up) | 0.20 | 5 | 25% | 10 min | 5.8 g |
| Camera backplate | `back-camera__black-PLA.stl` | black PLA | the **outer back face** | 0.20 | 5 | 25% | 1 h 35 min | 47.4 g |
| Plain backplate | `back-plain__black-PLA.stl` | black PLA | the **outer back face** | 0.20 | 5 | 25% | 57 min | 30.2 g |
| Lens ring (camera) | `lens-ring-camera__black-PLA.stl` | black PLA (gold optional) | the **ring's top face**, spigot up | 0.16 | 4 | 100% | 2 min | 0.5 g |
| Link cover (camera) | `link-cover-camera__black-PLA.stl` | black PLA | the **flat outer face**, hooks up | 0.16 | 4 | 30% | 10 min | 3.0 g |
| UnitV pins ×4 (2 + 2 spare) | `pins-camera__black-PLA.stl` | black PLA | **lying on their flats** (the split stands vertical) | 0.12 | 4 | 100% | 6 min | 0.9 g |
| **Fit test** | `fit-test__black-PLA.stl` | black PLA | as placed | 0.20 | 2 | 15% | **33 min** | 14.6 g |
| Chassis (both) | `chassis__white-PETG.stl` | white PETG | **already printed, not on the black plate** | | | | | |
| Plunger (camera) | `plunger-camera__white-PETG.stl` | white PETG | **already printed, not on the black plate** | | | | | |

**Total of the new black parts:** both backplates, bezel, small parts and fit test come to **about 3 h 34 min and 102 g**.

### Ready-to-print plate: `plates/black-PLA.3mf`

**One plate for the Creality SPARKX i7**, all in black PLA:

- the bezel;
- both backplates;
- the link cover;
- the lens ring;
- the UnitV pins.

It takes **about 3 h 54 min and 86.8 g**, brims included, as sliced by Creality Print 7.3.2. The fit test is not on it. Nor are the chassis and plunger, which are the white PETG parts.

**Settings.** The file uses Creality Print's own SPARKX i7 0.4 nozzle presets ("0.20mm Standard", "Generic PLA"). Each part keeps its layer height, walls and infill from the table above. On top of that:

- **Adhesion:**
  - first layer 0.25 mm thick and 0.44 mm wide, at 30 mm/s;
  - a 7 mm outer brim on the bezel and backplates, and 5 mm on the small parts, with a 0.1 mm gap so it peels off;
  - bed 60 °C; no fan for the first 2 layers, full fan from layer 4.
- **Supports:** none; revision 3 needs none. Organic tree supports from the build plate are set up in case a part ever needs them.
- **Stringing:**
  - 210 °C (215 °C first layer), flow capped at 11 mm³/s;
  - retraction 1.0 mm at 40 mm/s (the i7 is direct drive), wipe on, 0.2 mm z-hop;
  - avoid crossing walls, travel at 400 mm/s.
- **Speed:** outer walls 70 mm/s, inner walls 150, top 100, bridges 30. Overhang walls slow to 50, 30, 20 and 10 mm/s as they get steeper.
- **Look:** seam at the rear, gyroid infill, 4 bottom layers.

**To print:**

1. Clean the plate.
2. Open the file in Creality Print. Check that the printer is **SPARKX i7 0.4 nozzle** and that the filament points at the CFS slot holding the black PLA.
3. Press **Slice**, then **Print**.

**If a print fails:**

- **Parts lift or come loose:**
  - Wash the plate with dish soap and water, or wipe it with IPA, and don't touch it afterwards.
  - Watch the first layer: lines should be flat and touching, with no gaps or ridges. If not, re-run the printer's auto-levelling or adjust the Z offset.
  - If a brim corner still lifts, put a little glue stick under it.
- **Stringing, popping or a rough, bubbly surface:** the filament is damp. Dry PLA at 45-50 °C for 4-6 h.

**Rebuilding.** After a change to the model, run `py -3 -I plates/make_plates.py`.

- It needs numpy, plus Creality Print 7.3.2 and OrcaSlicer 2.5.2 unpacked in `plates/.tools/` (see the top of the script).
- It prints each part's overhang area. If that grows, re-check the part's support setting in `PARTS`.

## 2. Hardware

| Qty | Part | Goes | Notes |
| --- | --- | --- | --- |
| 2 | M2 × 4 countersunk (ISO 7046/10642) | Chassis into the device's two shallow holes (X28, X56) | **As before**, already in place if the chassis is fitted |
| 2 | M2 hex nut (4 mm across flats, 1.6 mm) | Captive in the chassis, bottom corners | **As before** |
| 2 | **M2 × 3 button head** (ISO 7380) | Backplate into the chassis nuts, bottom corners | Same screws for both backplates. The head sits in a counterbore over a 1.2 mm floor; the tip stops 0.4 mm short of the device; 1.25 mm of thread (3 turns) in the nut |
| 1 | M5Stack Grove (HY2.0-4P) cable, 10 cm | UnitV to the device's Grove port | About 80 mm of path. A longer cable works too: coil the slack under the channel lips |

**Fixings: what's used and why.**

- **Shallow holes.** The device's two shallow M2 holes (X28/X56, Ø2 × 3.5 mm) hold the chassis with M2 × 4. Both backplates then screw into the chassis's nuts with M2 × 3. That stays within your short screws.
- **Deep slots: why no screw tubes.** The other two holes (X14/X70) sit at the floor of 4.4 mm wide slots, 13.7 mm deep (measured in M5's STL). A tube that takes an M2 head needs at least 3.5 mm for the head plus 2 × 0.8 mm of wall, so 5.1 mm. That doesn't fit 4.4 mm. The chassis also already covers the slots, apart from 2.4 mm holes.
- **Slot hooks instead.** Each backplate has two **hooks that go into those slots** and click over the chassis's top edge. The back snaps into place and can't slide.
- **M5's own shell screws** (two through screws on the back) are never touched.

**Named parameters in `case.scad`:**

- **Screws:** `screw_len`, `tip_gap`, `screw_head`, `cb_d`. The counterbore depth and the floor are worked out from them, and OpenSCAD prints the thread engagement on export.
- **Hooks:** `slots`, `slot_w`, `hook_barb`.
- **Nut positions:** `fz_nuts` (frozen).

## 3. UnitV pins (camera)

**The holes.** The UnitV's mounting holes are LEGO-style: a **Ø4.85 bore straight through its 8.2 mm tail**, with a **Ø6.2 × 1.0 mm counterbore at both faces**. This was measured from M5's UnitV-M12 STL with `tools/measure_holes.py`.

**The pin** works like a LEGO friction pin:

- **Collar:** Ø6.0 × 0.9 mm, sits in the near counterbore.
- **UnitV end:** Ø4.7 shaft, 7.1 mm long, split 0.9 mm. Its barb (+0.3 mm per side, 0.8 mm long, square back) passes the 4.85 mm bore and **clicks into the far counterbore**. The pin reaches full depth.
- **Other end:** has the same barb, and clicks into a groove in the backplate's socket.
- **Printing:** the pins print **lying down**, so the layers run along the pin. That makes them strong in bending. Print spares.
- **Parameters:** `pin_clear` (0.15), `pin_barb` (0.30), `pin_slot` (0.9), `pin_flat` (0.35). Lengths and the groove position are derived from the measured hole.

## 4. Grove cable routing (camera)

**Size.** The pass-through in the backplate's right wall is **13 × 9 mm**:

- **Plug envelope:** 11 × 7 mm. You described your plug as "just ever so slightly wider than a USB-C port" and taller because of its two latch bumps.
  - A USB-C port is 8.9 × 3.3 mm.
  - The JST-PH-size 2.0 mm 4-way housing is 9.9 mm wide.
  - The board receptacle (LCSC C7429580) is 10 × 5.1 mm.
- **Margin:** 1 mm all round on top of that envelope.
- **Roof:** the opening has a 45° pointed top, so it prints without a bridge.
- **Channel entry:** inside, the channel's guide walls stop 5 mm short of the wall (`entry_len`). The plug, latch bumps included, therefore passes straight through the full 13 × 9 mm opening.

If your plug is bigger, measure it and set `plug` in `case.scad`.

**Inside the pod**, the cable has its own path:

- **From the UnitV.** It leaves the UnitV's far end (its Grove socket), U-turns there (about 8 mm of room), and runs up a **5 mm channel over the UnitV's thin tail**, between the two pin bosses.
- **Turn and exit.** It turns right on a 4 mm radius and leaves through the pass-through.
- **Clips.** Five **1.2 mm lips** clip it into the channel, so it doesn't rattle.
- **Covered.** The **link cover** clips into the pass-through with two hooks. It covers the cable from there up to the device's Grove plug, and the plug end has room to bend.

**Routing steps** (backplate on the table, inside up):

1. Lay the cable in the channel, pressing it under the lips. Leave the UnitV end at the bottom of the pod.
2. Push the other plug out through the pass-through from the inside.
3. Plug the cable into the UnitV.

## 5. Assembly

**Chassis (already done in the first build):**

1. Two M2 nuts in its hex pockets on the flat side.
2. Chassis on the device's back, camera opening over the IMX219.
3. Two M2 × 4 countersunk screws into X28/X56.

**Camera backplate:**

1. Press two pins into the pin bosses until they click. Drop the plunger into its guide, pin first.
2. Route the Grove cable (section 4) and plug it into the UnitV.
3. Lay the UnitV on the bosses, lens end up and lens over the round well. Press until both pins click into its holes.
4. Press the CardputerZero (with its chassis) into the backplate from the front until both slot hooks click over the chassis's top edge. Plug the Grove cable into the device's Grove port.
5. Fit the two M2 × 3 screws in the bottom corners.
6. Snap the bezel on from the front. Its tongue drops into the tray's step and clicks five times.
7. Clip the link cover over the pass-through.
8. Press the lens ring's spigot into the lens well. To swap the M12 lens, pull the ring off.

**Plain backplate:**

1. Press the CardputerZero (with its chassis) in from the front until both slot hooks click.
2. Fit the two M2 × 3 screws in the bottom corners.
3. Snap the bezel on.

**Swapping backplates:**

1. Unclip the bezel.
2. Remove the two M2 × 3 screws.
3. Lift the device out: rock it at the top to free the hooks.

The chassis and its screws stay on the device.

**USB-C2 note.** The device's Grove socket sits right under USB-C2. With a Grove cable plugged in, USB-C2 takes only slim plugs (about 5.5 mm thick or less). The case adds nothing to this.

## 6. Fit test (do this first, about 33 min)

`fit-test__black-PLA.stl`. Use it with your already-printed chassis fitted to the device.

| Piece | How to use it | Parameters it checks |
| --- | --- | --- |
| **Bezel** (the real part) | Press it onto the device's face. The lip should sit flat, clear of the keys and of the raised screen plate, with the cheek flat on the face left of the screen. | `lip_over`, `cheek`, `key_top`, `gap`, `dev_r` |
| **Left and right wall strips** (plain backplate walls) | Hold each against the device's side, floor under the chassis. Plug every cable in: USB-C1, USB-A, RJ45 with its latch, HDMI, TRRS, Grove, USB-C2. Reach the USB switch, the power slide and the side control. | `port_room`, `ctrl_room`, `gap`, window positions |
| **Top strip** (plain backplate's top edge, both slot hooks, camera window) | Push the device and chassis onto it. Both hooks should click into the slots over the chassis's top edge. The camera and IR windows should show fully. | `slots`, `slot_w`, `hook_barb`, `ir_win`, `imx_fov` |
| **Bottom strip** (both nut-screw seats) | Fit two M2 × 3 screws into the chassis nuts. The heads should sit below the surface and the screws should bite without touching the device. | `screw_len`, `tip_gap`, `fz_nuts` |
| **Two UnitV pins** | Push one through a UnitV hole. It should click and seat its collar in the counterbore. | `pin_clear`, `pin_barb` |

**Adjusting.** Edit the parameter at the top of `case.scad`, then run `python tools/build.py print`. `python tools/check_fit.py` reports any collision with the device, the chassis or the UnitV. `python tools/check_thin.py` reports any wall thinner than 1 mm.

| Symptom | Change |
| --- | --- |
| Device tight in the walls | `gap` += 0.05 to 0.1 |
| Device loose | `gap` -= 0.05 |
| Lip rubs keys or the screen plate | `lip_over` -= 0.2 |
| A plug's overmold rubs | `port_room` += 0.2 (or that window's rectangle) |
| Control hard to reach | `ctrl_room` += 0.5 |
| Hooks don't click, or too hard | `hook_barb` += / -= 0.2 |
| Bezel clicks too hard / too loose | `snap` -= / += 0.1 |
| Pin too tight / loose in the UnitV | `pin_clear` += / -= 0.05 |
| Pin doesn't click | `pin_barb` += 0.1 |
| Screw touches the device | `tip_gap` += 0.2 (seats move up) |
| Grove plug doesn't pass | Measure it and set `plug = [w, h, l]` |

## 7. Files

| Path | What |
| --- | --- |
| `case.scad` | Parametric model. Frozen chassis/plunger modules, print orientations (`*_print`), fit test, interference checks (`part="chk"`) |
| `stl/` | Print-ready STLs (section 1) |
| `plates/` | `black-PLA.3mf`, the ready-to-print SPARKX i7 plate, and `make_plates.py`, which builds and check-slices it |
| `renders/` | Renders of both backplates |
| `tools/build.py` | Exports the STLs. Never rewrites the frozen chassis or plunger |
| `tools/estimate.py` | Slices `stl/` with PrusaSlicer for times and grams |
| `tools/check_fit.py` | Collision check against the device, chassis and UnitV |
| `tools/check_thin.py` | Wall-thickness check: anything under 1 mm on every part (section 7) |
| `tools/measure_holes.py` | UnitV hole and device slot measurements |
| `tools/render.py`, `tools/finish.py` | Blender renders and the overview |
| `CONCEPT.md` | Measurements, sources and design rationale |

## 7. Wall-thickness check (`tools/check_thin.py`)

**How it works.** The check samples each part's surface and shoots a ray inward to measure the local wall. It reports any area thinner than 1.0 mm. The bezel and the lens ring come back clean.

These remaining flags are intentional:

| Part | Where | Thickness | Why it stays |
| --- | --- | --- | --- |
| Both backplates | Slot-hook barbs | 0.4-0.6 mm | The barb's 45° lead-in tip; the arm behind it is 1.2 × 3.6 mm |
| Camera backplate | Pin-socket floor | 1.0 mm | At the limit; 1.4 mm of skin under the socket |
| Link cover | Clip-hook barbs | 0.7 mm | Lead-in wedges on 1.2 mm fingers |
| UnitV pins | Collar, barbs, split | 0.9 mm collar; 0.3 mm barbs | The collar must fit the UnitV's 1.0 mm counterbore. The barbs and the 0.9 mm split are what make the pin click. |
| Chassis (frozen) | Nut-pocket shoulders and countersinks | 0.55-0.8 mm | Already printed in revision 2; not changed |
