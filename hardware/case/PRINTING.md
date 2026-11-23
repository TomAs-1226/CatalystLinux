# Printing and assembling the Catalyst case (revision 3)

One **bezel**, one **chassis** and either of two **backplates**.

## What changed after the first print

The first print failed. Revision 3 fixes each cause:

| Problem | Cause | Fix |
| --- | --- | --- |
| Frame turned to spaghetti | It stood 25.5 mm tall on about 1 mm of lip, and its port windows printed as 12-41 mm bridges | The frame is now a 4.9 mm bezel: the lip plus a flat cheek beside the screen. It prints face-down on a much bigger flat area. The side walls moved into the backplates, and every port window is a notch open at the top of the print on both parts. No bridges, no supports. |
| Back cover came out "weird" | Big shallow pockets on the face that prints on the bed: a 0.5 mm grip panel over 69 × 18 mm and a 0.8 mm trim pocket. Both print as bridges just above the first layers. A 1.5 mm inward ledge inside the lip, 14.6 mm up, needed a ring of supports. The pins were printed upright and weak. | Nothing is recessed on the bed face except bridgeable details: a 0.8 mm groove outlines the grip panel, and the counterbores are 4.2 mm. No inward ledges. The rear edges start with a 45° chamfer. Walls are 2 mm, ribs stiffen the pod, and screw towers have 1.7 mm walls. |
| UnitV pins didn't click | Short, and printed upright | Separate click pins, printed lying down |
| Grove plug didn't fit the cable opening | The opening was 3.8 × 8.6 mm | 12 × 8 mm, with a proper channel inside |

**Colour.** Everything new is **black PLA**. The gold trim, gold accent and blue link cover are gone:

- The lens ring and link cover stay separate parts because their shape needs it. The lens ring can still be printed in gold if you like.
- The plain backplate's accent is now a debossed line.

**Frozen parts.** The **chassis and plunger are unchanged**. They are the white PETG parts from the first print, and their STLs are byte-for-byte the same.

| | Camera backplate | Plain backplate |
| --- | --- | --- |
| Thickness | **40.5 mm** (42.9 at the lens ring) | **28.5 mm** |
| Footprint | 88.5 × 58.5 mm (94.1 wide with the link cover) | 88.5 × 58.5 mm |
| New black parts | 43.2 g, about 1 h 45 min | 26.9 g, about 55 min |
| Weight with the CardputerZero (95 g), chassis, UnitV, cable and screws | about 165 g | about 132 g |

## 1. Parts

All STLs in `stl/` are already oriented for the bed (z = 0 is the bed).

**Profile:** 0.4 mm nozzle, PLA at 210-215 °C, bed at 60 °C. **No supports.**

**Brims:**

- **Bezel:** a 5 mm brim. Its contact is a 3 mm ring plus the cheek.
- **Backplates:** a 3 mm brim on the corners if your bed lifts corners.

**Estimates:** from `tools/estimate.py` (PrusaSlicer 2.9.2, fast CoreXY profile). A bed-slinger at stock speeds takes about twice as long. Five walls make every 2 mm wall solid.

| Part | File | Material | **Print face (on the bed)** | Layer | Walls | Infill | Time | Filament |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Bezel (both versions) | `frame__black-PLA.stl` | black PLA | the **front face** (lip and cheek) | 0.20 | 5 | 25% | 6 min | 3.1 g |
| Camera backplate | `back-camera__black-PLA.stl` | black PLA | the **outer back face** | 0.20 | 5 | 25% | 1 h 24 min | 36.4 g |
| Plain backplate | `back-plain__black-PLA.stl` | black PLA | the **outer back face** | 0.20 | 5 | 25% | 49 min | 23.8 g |
| Lens ring (camera) | `lens-ring-camera__black-PLA.stl` | black PLA (gold optional) | the **ring's top face**, spigot up | 0.16 | 4 | 100% | 2 min | 0.5 g |
| Link cover (camera) | `link-cover-camera__black-PLA.stl` | black PLA | the **flat outer face**, hooks up | 0.16 | 4 | 30% | 8 min | 2.3 g |
| UnitV pins ×4 (2 + 2 spare) | `pins-camera__black-PLA.stl` | black PLA | **lying on their flats** (the split stands vertical) | 0.12 | 4 | 100% | 6 min | 0.9 g |
| **Fit test** | `fit-test__black-PLA.stl` | black PLA | as placed | 0.20 | 2 | 15% | **30 min** | 13.9 g |
| Chassis (both) | `chassis__white-PETG.stl` | white PETG | **already printed, not on the black plate** | | | | | |
| Plunger (camera) | `plunger-camera__white-PETG.stl` | white PETG | **already printed, not on the black plate** | | | | | |

**Total of the new black parts:** both backplates, bezel, small parts and fit test come to **about 3 h 04 min and 81 g**.

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

**Size.** The pass-through in the backplate's right wall is **12 × 8 mm**:

- **Plug envelope:** the HY2.0-4P plug is about 10 mm wide and 6 mm tall with its latch.
  - The width comes from the JST-PH-compatible 2.0 mm 4-way housing (9.9 mm).
  - The matching board receptacle is 10 × 5.1 mm (LCSC C7429580).
  - The latch adds about 1 mm.
- **Margin:** 1 mm all round on top of that envelope.
- **Roof:** the opening has a 45° pointed top, so it prints without a bridge.

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
6. Snap the bezel on from the front: five clicks.
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

## 6. Fit test (do this first, about 30 min)

`fit-test__black-PLA.stl`. Use it with your already-printed chassis fitted to the device.

| Piece | How to use it | Parameters it checks |
| --- | --- | --- |
| **Bezel** (the real part) | Press it onto the device's face. The lip should sit flat, clear of the keys and of the raised screen plate, with the cheek flat on the face left of the screen. | `lip_over`, `cheek`, `key_top`, `gap`, `dev_r` |
| **Left and right wall strips** (plain backplate walls) | Hold each against the device's side, floor under the chassis. Plug every cable in: USB-C1, USB-A, RJ45 with its latch, HDMI, TRRS, Grove, USB-C2. Reach the USB switch, the power slide and the side control. | `port_room`, `ctrl_room`, `gap`, window positions |
| **Top strip** (plain backplate's top edge, both slot hooks, camera window) | Push the device and chassis onto it. Both hooks should click into the slots over the chassis's top edge. The camera and IR windows should show fully. | `slots`, `slot_w`, `hook_barb`, `ir_win`, `imx_fov` |
| **Bottom strip** (both nut-screw seats) | Fit two M2 × 3 screws into the chassis nuts. The heads should sit below the surface and the screws should bite without touching the device. | `screw_len`, `tip_gap`, `fz_nuts` |
| **Two UnitV pins** | Push one through a UnitV hole. It should click and seat its collar in the counterbore. | `pin_clear`, `pin_barb` |

**Adjusting.** Edit the parameter at the top of `case.scad`, then run `python tools/build.py print`. `python tools/check_fit.py` reports any collision with the device, the chassis or the UnitV.

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
| `renders/` | Renders of both backplates |
| `tools/build.py` | Exports the STLs. Never rewrites the frozen chassis or plunger |
| `tools/estimate.py` | Slices `stl/` with PrusaSlicer for times and grams |
| `tools/check_fit.py` | Collision check against the device, chassis and UnitV |
| `tools/measure_holes.py` | UnitV hole and device slot measurements |
| `tools/render.py`, `tools/finish.py` | Blender renders and the overview |
| `CONCEPT.md` | Measurements, sources and design rationale |
