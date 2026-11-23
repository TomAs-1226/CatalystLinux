// Catalyst case for the M5Stack CardputerZero -- revision 3 (printable, strong, short screws).
//
// Parts (all black PLA unless frozen):
//   frame (bezel)      front lip + a flat cheek beside the screen, 4.9 mm tall: prints face-down
//   camera backplate   tray: back, all four side walls up to joint_z, UnitV K210 pod: back-down
//   plain backplate    tray: slim back and the same side walls: back-down
//   lens ring, link cover, UnitV pins (small separate parts)
//   chassis, plunger   FROZEN: already printed in white PETG from revision 2. Their modules
//                      below reproduce revision 2 exactly and are never re-exported.
//
// How it holds together:
//   - the chassis screws to the CardputerZero's two shallow M2 holes (X28, X56), M2 x 4 countersunk;
//   - either backplate screws to the chassis's two captive M2 nuts with M2 x 3 button heads
//     sunk in deep counterbores, and hooks into the device's two top slots (X14, X70)
//     where the hooks click over the chassis's top edge;
//   - the bezel clicks onto the backplate's side walls.
// Every port window reaches the bezel/tray joint, so it prints as an open notch on both parts.
//
// Coordinates (mm), measured from M5's own enclosure STL (CONCEPT.md):
//   X right (0 = left side, seen from the screen), Y up (0 = keyboard edge, 54 = top edge),
//   Z out of the screen (0 = device back, 22.1 = front face). Backplates sit at negative Z.
//
// Export: python tools/build.py   (or openscad -D 'part="frame_print"' -o x.stl case.scad)

part = "assembly_camera";
$fn = 64;

/* ===== 1. Device: measured (M5 STL, size drawing, docs; tools/measure_holes.py) ===== */
dev_w = 84;            // drawing 84.0
dev_h = 54;            // drawing 54.0
dev_t = 22.1;          // front face (screen plate rises to 23.1)
dev_r = 3.0;           // plan corner radius
imx   = [42.0, 47.8];  // IMX219 lens centre on the back
slots      = [[14, 50], [70, 50]];   // the two top slots: 4.4 mm wide (X 11.8-16.2 / 67.8-72.2),
slot_w     = 4.4;                    //   lower wall at Y 48.3, open to the top edge, 13.7 deep
slot_depth = 13.7;
raised     = [16.7, 29.8, 83.0, 52.0];  // raised screen plate on the face (x0 y0 x1 y1), Z 23.1

/* ===== 2. Best estimates (not published by M5). Generous where unsure. ===== */
key_top    = 23.2;                     // keycap tops (drawing: 23.2 overall)
imx_fov    = [62.2, 48.8];             // assumed standard IMX219 lens, H x V degrees
fov_margin = 2;                        // degrees added on every side
ir_win     = [30.0, 49.0, 54.0, 52.0]; // windows beside the IMX219 (x0 y0 x1 y1), taken as IR
side_ctrl  = [40.2, 2.0, 46.0, 5.4];   // recessed control, right side (y0 z0 y1 z1)
pwr_slide  = [8.6, 17.0, 14.4, 20.0];  // top slot taken as the power slide (x0 z0 x1 z1)
ctrl_room  = 1.2;                      // extra room around controls of unconfirmed purpose

/* ===== 3. UnitV K210 (measured from M5's UnitV-M12 STL) ===== */
k_size   = [24, 40, 12.7, 8.2, 17];  // width, length, head and tail thickness, head length
k_hole_d = 4.85;                     // LEGO-style hole straight through the 8.2 mm tail
k_cbore  = [6.2, 1.0];               // counterbore at both faces (diameter, depth)
k_holes  = [[4, 28.2], [20, 28.2]];  // in its own frame: x from its outer side, z from its lens end

/* ===== 4. Grove HY2.0-4P plug: the user's plug is "slightly wider than a USB-C port,
   and taller with two latch bumps" -- sized up from the JST-PH-size 2.0 mm 4-way housing ===== */
plug        = [11.0, 7.0, 9.0];  // width, height over the latch bumps, length with strain relief
plug_margin = 1.0;               // added all round for the pass-through

/* ===== 5. Fit: tune these from the fit test (PRINTING.md, "Fit test") ===== */
gap       = 0.25;   // device-to-wall clearance per side
lip_over  = 0.8;    // front lip overlap onto the device face
port_room = 0.0;    // grows every port window, per side
m2_clear  = 2.4;    // plain clearance hole for M2 (no thread in the plastic)
snap      = 0.30;   // bezel-to-tray snap bump height
hook_barb = 0.6;    // slot hooks: how far the barb reaches over the chassis's top edge
pin_clear = 0.15;   // UnitV pins: diametral clearance in the 4.85 mm hole
pin_barb  = 0.30;   // UnitV pins: barb height per side (clicks into the far counterbore)
pin_slot  = 0.9;    // UnitV pins: split width
pin_flat  = 0.35;   // UnitV pins: flat each side so they print lying down

/* ===== 6. Screws into the chassis nuts ===== */
screw_len  = 3;           // M2 x 3 button head (ISO 7380): both backplates, both nuts
screw_head = [3.5, 1.1];  // head diameter, height
tip_gap    = 0.4;         // screw tip stops this far short of the device back
cb_d       = 4.2;         // counterbore for the head

/* ===== 7. Case ===== */
wall       = 3.3;   // side walls: 2.0 mm full-height outer + 1.3 mm inner that steps down at the joint
skin       = 2.0;   // back skins
lip_t      = 1.6;   // front lip
joint_z    = 19.0;  // bezel / tray split, 3.1 mm below the device face
tongue_h   = 2.5;   // bezel tongue down into the tray's inner step
tongue_w   = 1.2;   // bezel tongue thickness
step_w     = tongue_w + 0.1;   // tray's inner step (the tray rim keeps a flat wall - step_w = 2.0 mm face)
front_edge = 1.0;   // bezel front edge radius (bed side: starts as a 45 deg chamfer)
edge_top   = 2.4;   // camera backplate rear-edge radius at the camera end
edge_bot   = 5.0;   // ... at the grip end (<= R_out)
plain_edge = 1.2;
cheek      = [16.3, 31.0];  // bezel cheek over the face left of the raised screen plate (x1, y0)

/* ===== FROZEN: revision 2 chassis and plunger (already printed, white PETG) ===== */
fz_chassis_t = 1.6;
fz_c_in      = 0.95;                      // chassis inset from the cavity outline
fz_nuts      = [[6, 6], [79, 6]];         // captive M2 nuts (hex 4.15 AF, from the device side)
fz_nut_af    = 4.15;
fz_boss      = [7.0, 0.6];                // nut bosses on the chassis's back face
fz_win       = [30.8, 45.07, 53.2];       // chassis camera window (open to the top edge)
fz_blind     = [[28, 50], [56, 50]];      // M2 x 4 countersunk into the device's shallow holes
fz_pocket    = [[14, 50], [70, 50]];      // 2.4 mm holes over the slots
k_x1         = 80.8;                      // UnitV outer long side: fixed by the frozen plunger
k_top        = 52.0;                      // UnitV lens end
fz_plunger   = [k_x1 - 24 - 1.4, 17.8];   // plunger axis (X, Y)

// ---------------------------------------------------------------- derived
R_cav   = dev_r + gap;               // 3.25
R_out   = R_cav + wall;              // 5.25 (concentric: outer = inner + wall)
X0 = -gap - wall;  X1 = dev_w + gap + wall;
Y0 = -gap - wall;  Y1 = dev_h + gap + wall;
lip_z0  = dev_t + 0.2;
front_z = lip_z0 + lip_t;            // 23.9
Zch     = -fz_chassis_t;             // chassis back face (-1.6)
k_x0    = k_x1 - k_size[0];
k_bot   = k_top - k_size[1];
k_back  = Zch;                       // UnitV sits on the chassis
k_tail  = k_back - k_size[3];        // its tail front face (-9.8)
lens    = [k_x1 - 12, k_top - 9];    // M12 lens centre (68.8, 43)
Zc      = k_back - k_size[2] - 0.3 - skin;   // camera back surface (-16.6): set by the UnitV
seat_z  = -tip_gap - screw_len;      // head seat for the nut screws (-3.4)
floor_t = (Zch - fz_boss[1]) - seat_z;       // plastic under the head (1.2)
Zp      = seat_z - screw_head[1] - 0.1;      // plain back surface (-4.6): head sinks below it
pass    = [plug[0] + 2 * plug_margin, plug[1] + 2 * plug_margin];  // 13 x 9
pass_y  = 30.5;                      // pass-through centre (Y): channel stays under the UnitV tail
entry_len = 5.0;                     // channel guides stop this far from the wall: open plug entry
pass_z  = [Zc + skin + 1.9, Zc + skin + 1.9 + pass[1]];  // pass-through Z span (-12.7 .. -3.7), clear of the rear-edge round
k_pins  = [for (h = k_holes) [k_x1 - h[0], k_top - h[1]]];
function fov_x(d) = 1.5 + d * tan(imx_fov[0] / 2 + fov_margin);
function fov_y(d) = 1.5 + d * tan(imx_fov[1] / 2 + fov_margin);
bay = [imx[0] - fov_x(-Zc) - 1.0, imx[1] - fov_y(-Zc) - 1.2, imx[0] + fov_x(-Zc) + 0.6];

echo(str("thickness: camera ", front_z - Zc, " mm (+2.4 lens ring), plain ", front_z - Zp, " mm"));
echo(str("nut screws M2 x ", screw_len, ": ", screw_len + seat_z + 1.65, " mm in the nut, ", floor_t,
         " mm floor; pass-through ", pass[0], " x ", pass[1], " mm"));
assert(floor_t >= 1.2 - 0.001, "floor under the screw heads below 1.2 mm");
assert(front_z >= key_top + 0.5, "front lip must stand proud of the keys");
assert(bay[2] + 1.2 <= k_x0 - 0.2, "bay wall would hit the UnitV");

// ---------------------------------------------------------------- helpers
module r2(a0, b0, a1, b1) translate([a0, b0]) square([a1 - a0, b1 - b0]);
module rr2d(x0, y0, x1, y1, r)
    translate([x0 + r, y0 + r]) offset(r = r) square([x1 - x0 - 2 * r, y1 - y0 - 2 * r]);
module round2d(r) offset(r = r) offset(delta = -r) offset(r = -r) offset(delta = r) children();
module capsule2d(a0, b0, a1, b1) rr2d(a0, b0, a1, b1, min(a1 - a0, b1 - b0) / 2 - 0.001);
module cav2d(o = 0) rr2d(-gap - o, -gap - o, dev_w + gap + o, dev_h + gap + o, R_cav + o);
// Disc of radius R, height h, rounded bottom (eb) and top (et) edges; chb/cht turn the
// first part of the bed-side round into a 45 deg chamfer so it prints without droop.
module puck(R, h, eb, et, chb = false, cht = false)
    rotate_extrude() intersection() {
        square([R, h]);
        hull() {
            square([0.01, h]);
            if (eb > 0) translate([R - eb, eb]) circle(eb); else square([R, 0.01]);
            if (eb > 0 && chb) translate([R - 0.586 * eb, 0]) square(0.01);
            if (et > 0) translate([R - et, h - et]) circle(et); else translate([0, h - 0.01]) square([R, 0.01]);
            if (et > 0 && cht) translate([R - 0.586 * et, h - 0.01]) square(0.01);
        }
    }
module softbox2(x0, y0, x1, y1, z0, z1, R, eb_bot, eb_top, et, chb = false, cht = false)
    hull() for (x = [x0 + R, x1 - R]) {
        translate([x, y0 + R, z0]) puck(R, z1 - z0, eb_bot, et, chb, cht);
        translate([x, y1 - R, z0]) puck(R, z1 - z0, eb_top, et, chb, cht);
    }
module ext_x(x, len) translate([x, 0, 0]) rotate([90, 0, 90]) linear_extrude(len) children(); // 2D(Y,Z) -> +X
module ext_y(y, len) translate([0, y, 0]) rotate([90, 0, 0]) linear_extrude(len) children();  // 2D(X,Z) -> -Y
module grow() offset(delta = port_room) children();

// ---------------------------------------------------------------- FROZEN parts (revision 2, do not edit)
module chassis() {
    difference() {
        union() {
            translate([0, 0, -fz_chassis_t]) linear_extrude(fz_chassis_t)
                rr2d(fz_c_in, fz_c_in, dev_w - fz_c_in, dev_h - fz_c_in, R_cav - gap - fz_c_in);
            for (p = fz_nuts) translate([p[0], p[1], -fz_chassis_t - fz_boss[1]]) cylinder(d = fz_boss[0], h = fz_boss[1] + 0.01);
        }
        translate([0, 0, -fz_chassis_t - 1]) linear_extrude(fz_chassis_t + 2) rr2d(fz_win[0], fz_win[1], fz_win[2], 60, 2.2);
        for (p = fz_blind) translate([p[0], p[1], -fz_chassis_t - 1]) {
            cylinder(d = 2.4, h = 5);
            translate([0, 0, 1]) cylinder(d1 = 4.2, d2 = 2.4, h = 0.9);
        }
        for (p = fz_pocket) translate([p[0], p[1], -fz_chassis_t - 1]) cylinder(d = 2.4, h = 5);
        for (p = fz_nuts) translate([p[0], p[1], 0]) {
            translate([0, 0, -1.65]) cylinder(d = fz_nut_af / cos(30), h = 2, $fn = 6);
            translate([0, 0, -3]) cylinder(d = 2.4, h = 4);
        }
        translate([0, 0, -fz_chassis_t - 1]) linear_extrude(fz_chassis_t + 2) rr2d(10.4, 24.0, 23.6, 26.0, 0.9);
    }
}
module plunger()   // pin + wedge for the UnitV's inner button (rev 2: skin 1.6, back at -16.2)
    translate([fz_plunger[0], fz_plunger[1], 0]) {
        translate([0, 0, -16.6]) cylinder(d = 2.0, h = 2.2, $fn = 24);
        translate([-1.0, -1.5, -14.6]) cube([2.0, 3.0, 9.0]);
        hull() {
            translate([-1.0, -1.5, -5.6]) cube([2.0, 3.0, 0.01]);
            translate([-1.0, -1.5, -3.6]) cube([2.3, 3.0, 0.01]);
        }
    }

// ---------------------------------------------------------------- port windows (cut through bezel and trays)
jt = joint_z + 0.01;   // windows that must stay open through the joint reach at least here
module left_windows()  // RJ45, USB switch, USB-A, USB-C1 -- plug overmolds included
    grow() round2d(1.6) union() {
        r2(4.2, 1.4, 22.0, 21.0);         // RJ45 (jack Y5.0-21.2 Z2.2-16.8) + boot, up through the joint
        r2(21.0, 12.6, 33.6, 21.0);       // between RJ45 and USB, above the switch's bottom
        r2(23.2, 12.6, 31.8, 21.0);       // USB switch (Y24.0-31.0 Z13.4-20.2)
        r2(32.6, 8.0, 47.4, 21.6);        // USB-A (Y33.4-46.6 Z8.8-16.0) + USB-C1 (Y35.6-44.4 Z17-20.2)
    }
// HDMI type A (Y5.8-20.6 Z9.8-15.4), TRRS (Y25-30 Z11.4-16.8), Grove (Y34-46 Z10.6-17), USB-C2 (Y35.5-44.5 Z17-20.2)
// The recessed side control (Y40.2-46 Z2-5.4) joins it: the web between them would be 0.4 mm.
module right_windows() grow() round2d(1.6) union() {
    rr2d(3.0, 7.0, 47.4, 21.6, 2.5);
    capsule2d(side_ctrl[0] - ctrl_room, side_ctrl[1] - ctrl_room, side_ctrl[2] + ctrl_room, side_ctrl[3] + ctrl_room);
    r2(side_ctrl[0] - ctrl_room, side_ctrl[3], side_ctrl[2] + ctrl_room, 8.0);
}
// power slide (X8.6-14.4 Z17-20), microSD (X18.2-29.8 Z17-18.8), EXT 2x7 (X32.8-51.2 Z10.4-16.8),
// slot (X37.8-46 Z18.6-19.6), both vent rows (Z7.2-9.8): one window, nothing left hanging
module top_windows() grow() rr2d(pwr_slide[0] - 2.2 - ctrl_room, 6.6, 63.8, 21.4, 1.2);
module all_windows() {
    ext_x(X0 - 1, wall + gap + 1.5) left_windows();
    ext_x(dev_w - 0.5, wall + gap + 1.5) {
        right_windows();
        capsule2d(13.2, 0.4, 21.8, 5.0);   // three slots at the back-right corner
    }
    ext_y(Y1 + 1, wall + gap + 1.5) { top_windows(); grow() union() { capsule2d(74.6, 15.0, 77.4, 18.0); r2(74.6, 16.4, 77.4, jt + 0.6); } }  // pinhole: square through the joint
    ext_y(0.5, wall + gap + 1.5) grow() union() {                                                     // speaker
        capsule2d(29.6, 10.0, 54.4, 16.4);
        r2(29.6, 13.2, 54.4, jt);
    }
}

// ---------------------------------------------------------------- bezel / tray joint
// The tray's rim is a flat face, wall - step_w = 2.0 mm wide. Its inner 1.3 mm steps down
// tongue_h, and the bezel's 1.2 mm tongue drops into that step; snap bumps on the tongue's
// outer face click into dimples in the step's 2.0 mm wall.
snap_pts = [[15, -gap - tongue_w], [69, -gap - tongue_w], [70, dev_h + gap + tongue_w],
            [-gap - tongue_w, 50], [dev_w + gap + tongue_w, 50]];
module bezel_tongue()
    difference() {
        union() {
            translate([0, 0, joint_z - tongue_h + 0.1]) linear_extrude(tongue_h - 0.1 + 0.01)
                difference() { cav2d(tongue_w); cav2d(0); }
            for (p = snap_pts) translate([p[0], p[1], joint_z - 1.6]) sphere(r = snap + 0.15, $fn = 24);
        }
        translate([0, 0, joint_z - tongue_h - 1]) linear_extrude(tongue_h + 2) cav2d(0);
    }
module tray_step() {    // cut from both trays
    translate([0, 0, joint_z - tongue_h]) linear_extrude(tongue_h + 1) cav2d(step_w);
    for (p = snap_pts) translate([p[0], p[1], joint_z - 1.6]) sphere(r = snap + 0.25, $fn = 24);
}

// ---------------------------------------------------------------- bezel (frame)
module opening2d()   // keyboard + raised screen plate, minus the flat cheek at the top left
    let(o = lip_over, Ro = R_out - (wall + gap + lip_over))
    round2d(2.2) difference() {
        rr2d(o, o, dev_w - o, dev_h - o, Ro);
        r2(-1, cheek[1], cheek[0], dev_h + 1);
    }
module frame() {
    difference() {
        union() {
            softbox2(X0, Y0, X1, Y1, joint_z, front_z, R_out, 0, 0, front_edge, cht = true);
            bezel_tongue();
        }
        translate([0, 0, joint_z - tongue_h - 1]) linear_extrude(lip_z0 - joint_z + tongue_h + 1) cav2d(0);
        translate([0, 0, lip_z0 - 1]) linear_extrude(front_z - lip_z0 + 1.2) opening2d();
        for (i = [0 : 3])   // 0.4 mm thumb chamfer at 45 deg (leaves a 1.2 mm lip edge), 0.1 mm steps
            translate([0, 0, front_z - 0.4 + i * 0.1]) linear_extrude(0.11 + (i == 3 ? 0.1 : 0)) offset(delta = (i + 1) * 0.1) opening2d();
        all_windows();
    }
}

// ---------------------------------------------------------------- shared tray pieces
// Slot hook: rises past the chassis's top edge into the device's top slot and clicks over
// the chassis's front face (the frozen chassis covers the slots, so the hook grips it).
module slot_hook(x)
    translate([x, 0, 0]) {
        translate([-1.8, dev_h - fz_c_in + 0.1, -3.0]) cube([3.6, 1.2, 3.0 + 1.25]);             // arm, 1.2 mm
        hull() {                                                                                 // barb, 45 deg lead-in
            translate([-1.8, dev_h - fz_c_in + 0.1 - hook_barb, 0.05]) cube([3.6, hook_barb + 1.2, 0.4]);
            translate([-1.8, dev_h - fz_c_in + 0.1, 0.05 + 0.4 + hook_barb]) cube([3.6, 1.2, 0.01]);
        }
        translate([-1.8, dev_h - fz_c_in + 0.1, -3.0]) cube([3.6, dev_h + gap - (dev_h - fz_c_in + 0.1) + 0.6, 1.2]);  // root into the top wall
    }
module hook_relief(x)    // lets the arm spring outward; the top wall keeps wall - 1.6 = 1.7 mm behind it
    translate([x - 2.2, dev_h + gap - 0.01, -1.8]) cube([4.4, 1.6, 3.6]);
module nut_screw_cut(top_z, p) translate([p[0], p[1], 0]) {
    translate([0, 0, top_z - 1]) cylinder(d = cb_d, h = seat_z - top_z + 1);
    translate([0, 0, seat_z - 0.01]) cylinder(d = m2_clear, h = 6);
    translate([0, 0, Zch - fz_boss[1] - 0.05]) cylinder(d = fz_boss[0] + 0.6, h = fz_boss[1] + 0.06);   // clears the nut boss
}
module grip_groove(zs)   // outline of the grip panel: a 0.8 mm groove, not a recess
    translate([0, 0, zs - 1]) linear_extrude(1.4) difference() {
        rr2d(7.5, 9.5, 76.5, 27.5, 4.0);
        rr2d(8.3, 10.3, 75.7, 26.7, 3.2);
    }

// ---------------------------------------------------------------- camera backplate
module bay_shape(z, grow = 0)    // bay outline at height z (chassis level -> back surface)
    let(t = min(1, max(0, (Zch - z) / (Zch - Zc))))
    rr2d(fz_win[0] + (bay[0] - fz_win[0]) * t - grow, fz_win[1] + (bay[1] - fz_win[1]) * t - grow,
         fz_win[2] + (bay[2] - fz_win[2]) * t + grow, 66, 2.2 + 1.8 * t + grow);
module bay_cut() {
    translate([0, 0, Zch - 1.0]) linear_extrude(1.01) bay_shape(Zch);   // square top edge
    hull() {
        translate([0, 0, Zch - 1.0]) linear_extrude(0.01) bay_shape(Zch);
        translate([0, 0, Zc - 0.01]) linear_extrude(0.01) bay_shape(Zc);
    }
    translate([0, 0, Zc - 6]) linear_extrude(6) bay_shape(Zc);
}
module bay_sleeve()   // 2 mm walls, except toward the UnitV (fixed by the frozen plunger): >= 1.5
    intersection() {
        union() {
            translate([0, 0, Zch - 1.0]) linear_extrude(1.0) bay_shape(Zch, skin);
            hull() {
                translate([0, 0, Zch - 1.0]) linear_extrude(0.01) bay_shape(Zch, skin);
                translate([0, 0, Zc + 0.5]) linear_extrude(0.01) bay_shape(Zc, skin);
            }
        }
        translate([X0, Y0, Zc]) cube([k_x0 - 0.3 - X0, Y1 - Y0, -Zc]);
    }
module cam_shell(z1 = joint_z)
    softbox2(X0, Y0, X1, Y1, Zc, z1, R_out, edge_bot, edge_top, 0, chb = true);
module channel2d(o = 0)          // Grove cable path over the UnitV tail: up the middle, turn right
    round2d(4.0) union() {
        r2(lens[0] - 2.5 - o, 3.0 - o, lens[0] + 2.5 + o, pass_y + 2.5 + o);
        r2(lens[0] - 2.5 - o, pass_y - 2.5 - o, X1, pass_y + 2.5 + o);
    }
pin_sock_bot = Zc + skin - 0.6;  // pin socket floor (1.4 mm of skin left)
module camera_back() {
    difference() {
        union() {
            difference() {
                cam_shell();
                softbox2(X0 + wall, Y0 + wall, X1 - wall, Y1 - wall, Zc + skin, joint_z + 1, R_cav,
                         edge_bot - skin, edge_top - skin, 0);
            }
            intersection() {
                union() {
                    bay_sleeve();
                    // ribs: stiffen the grip area and carry the chassis
                    for (y = [12, 30]) translate([X0 + 1, y - 1, Zc + 1]) cube([55 - X0, 2, Zch - Zc - 1.1]);
                    for (x = [30, 46]) translate([x - 1, Y0 + 1, Zc + 1]) cube([2, 31 - Y0, Zch - Zc - 1.1]);
                    // screw towers over the chassis nuts
                    for (p = fz_nuts) translate([p[0], p[1], Zc + 1]) cylinder(d = cb_d + 3.4, h = Zch - fz_boss[1] - Zc - 1);
                    // UnitV pin bosses (sockets for the separate click pins)
                    for (p = k_pins) translate([p[0], p[1], Zc + 1]) cylinder(d = 9, h = k_tail - 0.2 - Zc - 1);
                    // cable channel guides (3.5 tall)
                    translate([0, 0, Zc + skin - 0.01]) linear_extrude(3.5) difference() {
                        channel2d(1.6); channel2d(0);
                        translate([X1 - wall - entry_len, pass_y - pass[0] / 2]) square([entry_len + 1, pass[0]]);  // open entry
                    }
                    // guide for the frozen plunger (open toward the UnitV)
                    translate([fz_plunger[0], fz_plunger[1], Zc + 1]) difference() {
                        translate([-2.6, -3.2, 0]) cube([3.6, 6.4, 11]);
                        translate([-1.15, -1.65, -1]) cube([2.3, 3.3, 13]);
                    }
                }
                cam_shell(Zch);
            }
            // retaining lips over the cable channel (1.2 mm overhangs)
            for (y = [9, 19]) {
                translate([lens[0] + 2.5 - 1.2, y, Zc + skin + 2.3]) cube([2.8, 3, 1.2]);
                translate([lens[0] - 2.5 - 1.6, y, Zc + skin + 2.3]) cube([2.8, 3, 1.2]);
            }
            translate([X1 - wall - entry_len - 4, pass_y + 2.5 - 1.2, Zc + skin + 2.3]) cube([3, 2.8, 1.2]);
            for (s = slots) slot_hook(s[0]);
        }
        bay_cut();
        for (p = fz_nuts) nut_screw_cut(Zc, p);
        tray_step();
        translate([lens[0], lens[1], Zc - 1]) cylinder(d = 18.0, h = skin + 3);    // M12 lens well
        grip_groove(Zc);
        all_windows();
        for (s = slots) hook_relief(s[0]);
        // pin sockets: the bore, and a groove at the bottom for the barb
        for (p = k_pins) translate([p[0], p[1], 0]) {
            translate([0, 0, pin_sock_bot]) cylinder(d = k_hole_d, h = 20);
            translate([0, 0, pin_groove_z - pin_barb_len - 0.1]) cylinder(d = k_hole_d + 2 * pin_barb + 0.1, h = pin_barb_len + 0.1);
        }
        // Grove pass-through: plug housing + 1 mm all round, 45 deg roof (no bridge)
        ext_x(X1 - wall - 1, wall + 2) hull() {
            rr2d(pass_y - pass[0] / 2, pass_z[0], pass_y + pass[0] / 2, pass_z[1], 1.5);
            translate([pass_y - 0.5, pass_z[1] + pass[0] / 2 - 1]) square([1, 0.5]);
        }
        // UnitV microSD (its lens end) through the top face: straight, 0.8 mm outer chamfer
        ext_y(Y1 + 1, wall + 2) rr2d(k_x1 - 17.4, k_back - 7.4, k_x1 - 4.4, k_back - 2.4, 2.0);
        hull() {
            ext_y(Y1 - 0.8, 0.01) rr2d(k_x1 - 17.4, k_back - 7.4, k_x1 - 4.4, k_back - 2.4, 2.0);
            ext_y(Y1 + 0.01, 0.01) rr2d(k_x1 - 18.2, k_back - 8.2, k_x1 - 3.6, k_back - 1.6, 2.8);
        }
        // UnitV USB-C tunnel to the bottom face
        ext_y(k_bot + 0.5, 20) rr2d(k_x1 - 19.8, k_back - 8.7, k_x1 - 5.8, Zch - 0.3, 3.0);
        // button A: flexure key in the right wall (U slot, hinge at the bottom)
        ext_x(X1 - wall - 1, wall + 2) difference() {
            rr2d(k_top - 38.6, k_back - 7.4, k_top - 30.0, k_back - 1.0, 2.2);
            rr2d(k_top - 37.8, k_back - 6.6, k_top - 30.8, k_back - 1.8, 1.6);
            translate([k_top - 37.8, k_back - 7.4]) square([7.0, 0.8]);
        }
        // button B: hole for the frozen plunger's pin
        translate([fz_plunger[0], fz_plunger[1], Zc - 1]) cylinder(d = 2.3, h = skin + 2, $fn = 24);
    }
    // button A nub on the flexure tab
    translate([k_x1 + 0.3, k_top - 34.2 - 1.5, k_back - 4.3 - 1.5]) cube([X1 - wall - k_x1 - 0.29, 3, 3]);
}

// ---------------------------------------------------------------- plain backplate
plain_win = [fz_win[0], fz_win[1], fz_win[2], ir_win[3] + 0.6];
accent    = [35.0, 40.4, 49.0, 42.4];   // debossed capsule under the window
module plain_back() {
    difference() {
        union() {
            difference() {
                softbox2(X0, Y0, X1, Y1, Zp, joint_z, R_out, plain_edge, plain_edge, 0, chb = true);
                translate([0, 0, Zch]) linear_extrude(joint_z + 2) cav2d(0);
            }
            for (s = slots) slot_hook(s[0]);
        }
        translate([0, 0, Zp - 1]) linear_extrude(-Zp + 2) rr2d(plain_win[0], plain_win[1], plain_win[2], plain_win[3], 2.2);
        translate([0, 0, Zp - 1]) linear_extrude(1.4) capsule2d(accent[0], accent[1], accent[2], accent[3]);
        grip_groove(Zp);
        translate([0, 0, Zp - 1]) linear_extrude(-Zp + 2) rr2d(10.6, 24.4, 23.4, 25.6, 0.6);  // vent over the slit
        for (p = fz_nuts) nut_screw_cut(Zp, p);
        tray_step();
        for (s = slots) hook_relief(s[0]);
        all_windows();
    }
}

// ---------------------------------------------------------------- separate small parts
// UnitV click pin: one end into the backplate's socket, the other through the UnitV's 8.2 mm
// tail; each barb clicks behind a bore (socket groove / the UnitV's far counterbore), like a LEGO pin.
pin_d        = k_hole_d - pin_clear;                 // 4.70
pin_collar   = [k_cbore[0] - 0.2, k_cbore[1] - 0.1]; // 6.0 x 0.9, sits in the UnitV's near counterbore
pin_barb_len = 0.8;
pin_len_uv   = (k_size[3] - k_cbore[1] + 0.05) - (pin_collar[1] + 0.05) + pin_barb_len;  // 7.1
pin_len_sock = (k_tail - 0.05) - pin_sock_bot - 0.3;                                    // into the socket
pin_groove_z = k_tail + 0.05 - (pin_len_sock - pin_barb_len) + 0.1;                     // just above the barb's back face
module pin_half(L)               // along +Z from 0: shaft, barb at the tip, split
    difference() {
        union() {
            cylinder(d = pin_d, h = L - pin_barb_len, $fn = 48);
            translate([0, 0, L - pin_barb_len]) cylinder(d1 = pin_d + 2 * pin_barb, d2 = pin_d - 0.4, h = pin_barb_len, $fn = 48);
        }
        translate([-pin_slot / 2, -5, L - min(L - 0.8, 5.5)]) cube([pin_slot, 10, 10]);
    }
module pin()                     // collar at z = 0, UnitV end up (+Z), socket end down
    intersection() {
        union() {
            translate([0, 0, -pin_collar[1] / 2]) cylinder(d = pin_collar[0], h = pin_collar[1], $fn = 48);
            translate([0, 0, pin_collar[1] / 2 - 0.01]) pin_half(pin_len_uv);
            mirror([0, 0, 1]) translate([0, 0, pin_collar[1] / 2 - 0.01]) pin_half(pin_len_sock);
        }
        translate([-10, -(pin_d / 2 - pin_flat), -20]) cube([20, pin_d - 2 * pin_flat, 40]);   // flats
    }
module lens_ring()               // around the M12 lens; spigot into the lens well
    difference() {
        union() {
            translate([lens[0], lens[1], Zc - 2.4]) cylinder(d = 21.6, h = 2.4 + 0.01);
            translate([lens[0], lens[1], Zc]) cylinder(d = 17.6, h = 1.4);
        }
        translate([lens[0], lens[1], Zc - 5]) cylinder(d = 15.2, h = 10);
        translate([lens[0], lens[1], Zc - 2.41]) cylinder(d1 = 19.2, d2 = 15.2, h = 2.0);
    }
module soft_bar(y0, y1, z0, z1, p, r = 1.6)
    hull() for (y = [y0 + r, y1 - r], z = [z0 + r, z1 - r]) {
        translate([X1 + p - r, y, z]) sphere(r = r, $fn = 32);
        translate([X1, y, z]) rotate([0, 90, 0]) cylinder(r = r, h = 0.01, $fn = 32);
    }
// Link cover: closes over the Grove cable between the pass-through and the device's
// Grove plug; two hooks clip into the pass-through; a window keeps the side control reachable.
lc = [pass_y - pass[0] / 2 - 2.0, 50.0, pass_z[0] - 1.0, 10.5, 6.0];   // y0 y1 z0 z1 protrusion (clear of the button-A key)
module link() {
    cy = [pass_y - pass[0] / 2 + 1.4, pass_y + pass[0] / 2 - 1.4];   // cable space between the hooks
    difference() {
        soft_bar(lc[0], lc[1], lc[2], lc[3], lc[4]);
        translate([X1 - 50, -50, -50]) cube([50, 200, 200]);                      // flat inner face
        // cable space: up from the pass-through, widening at the top for the plug's bend
        translate([X1 - 1, cy[0], pass_z[0] + 0.5]) cube([lc[4] - 2.4 + 1, cy[1] - cy[0], lc[3] - pass_z[0]]);
        translate([X1 - 1, cy[0], side_ctrl[3] + 2.2 + 1.2]) cube([lc[4] - 2.4 + 1, lc[1] - 2.4 - cy[0], 5]);  // plug-bend space, above the control window
        hull() {   // window onto the side control, chamfered for a fingertip
            translate([X1 - 1, 0, 0]) ext_x(0, 0.01) capsule2d(side_ctrl[0] - 0.6, side_ctrl[1] - 1.0, side_ctrl[2] + 0.6, side_ctrl[3] + 1.0);
            translate([X1 + lc[4] + 0.01, 0, 0]) ext_x(0, 0.01) capsule2d(side_ctrl[0] - 1.4, side_ctrl[1] - 1.8, side_ctrl[2] + 1.4, side_ctrl[3] + 1.8);
        }
    }
    // two clip hooks at the pass-through's short edges: fingers into the opening, barbs behind the wall
    for (sd = [-1, 1]) {
        yedge = pass_y + sd * pass[0] / 2;          // the opening's edge
        yf = yedge - sd * 0.05 - (sd < 0 ? 0 : 1.2);  // finger, just inside the edge
        translate([X1 - wall - 0.7, yf, pass_z[0] + 1.6]) cube([wall + 0.71, 1.2, pass[1] - 3.2]);
        hull() {                                    // barb with a lead-in
            translate([X1 - wall - 0.7, sd < 0 ? yf - 0.5 : yf + 1.2, pass_z[0] + 1.6]) cube([0.7, 0.5, pass[1] - 3.2]);
            translate([X1 - wall + 0.4, sd < 0 ? yf : yf + 1.19, pass_z[0] + 1.6]) cube([0.01, 0.01, pass[1] - 3.2]);
        }
    }
}

// ---------------------------------------------------------------- fit test
module fit_bezel() frame();
module fit_side(left)    // outer 2 mm of a plain-tray side wall with its port windows (the bezel prints whole)
    intersection() {
        plain_back();
        translate([left ? X0 - 1 : X1 - 2.0, Y0 - 1, Zp - 1]) cube([3.0, Y1 - Y0 + 2, joint_z - Zp + 1.01]);   // outer 2 mm of the wall: window positions
    }
module fit_top()         // top edge of the plain backplate: both slot hooks + the camera window
    intersection() { plain_back(); translate([X0 - 1, 46.5, Zp - 1]) cube([X1 - X0 + 2, 20, -Zp + 1 + 2.5]); }
module fit_bottom()      // the two nut-screw seats (M2 x 3) of the plain backplate
    intersection() {
        plain_back();
        for (p = fz_nuts) translate([p[0] - 6.5, Y0 - 1, Zp - 1]) cube([13, p[1] + 7 - Y0 + 1, -Zp + 1]);
    }

// ---------------------------------------------------------------- render-only dummies
module key() hull() for (dx = [-1.0, 1.0]) translate([dx, 0, 0]) cylinder(d = 2.3, h = 1.55, $fn = 24);
module keys_all() {
    for (i = [0 : 10], y = [3.5, 10.0, 16.5, 23.0]) translate([4.5 + 7.5 * i, y, dev_t - 0.6]) key();
    for (x = [4.5, 79.5]) translate([x, 29.5, dev_t - 0.6]) key();
}
module lenses() translate([lens[0], lens[1], k_back - k_size[2] - 3.7]) difference() {
    cylinder(d = 14, h = 3.7);
    translate([0, 0, -0.01]) cylinder(d = 8.6, h = 0.5);
}
module glass_imx() translate([imx[0], imx[1], -0.5]) cylinder(d = 5.2, h = 0.5);
module glass_k210() translate([lens[0], lens[1], k_back - k_size[2] - 3.7 - 0.3]) cylinder(d = 8.4, h = 0.8);
module screw_heads(top_z)        // button heads in the nut counterbores, render only
    for (p = fz_nuts) translate([p[0], p[1], seat_z - screw_head[1]]) difference() {
        cylinder(d = screw_head[0], h = screw_head[1], $fn = 32);
        translate([0, 0, -0.01]) cylinder(d = 1.4, h = 0.6, $fn = 6);
    }
module pins_in_place() for (p = k_pins) translate([p[0], p[1], k_tail + 0.05 + pin_collar[1] / 2]) pin();

// ---------------------------------------------------------------- print orientation
module on_bed(z) translate([0, 0, -z]) children();
module flip(ztop) translate([0, 0, ztop]) rotate([180, 0, 0]) children();
module pin_flat() translate([0, 0, pin_d / 2 - pin_flat]) rotate([90, 0, 0]) pin();   // lying on a flat

// ---------------------------------------------------------------- output
if (part == "frame")          frame();
if (part == "chassis")        chassis();
if (part == "camera_back")    camera_back();
if (part == "plain_back")     plain_back();
if (part == "lens_ring")      lens_ring();
if (part == "link")           link();
if (part == "plunger")        plunger();
if (part == "pins")           pins_in_place();
if (part == "keys")           keys_all();
if (part == "lenses")         lenses();
if (part == "glass_imx")      glass_imx();
if (part == "glass_k210")     glass_k210();
if (part == "screw_heads")    screw_heads();
// oriented for the bed (z = 0 is the bed)
if (part == "frame_print")       flip(front_z) frame();                       // face down
if (part == "camera_back_print") on_bed(Zc) camera_back();                    // back face down
if (part == "plain_back_print")  on_bed(Zp) plain_back();                     // back face down
if (part == "lens_ring_print")   on_bed(Zc - 2.4) lens_ring();                // ring face down, spigot up
if (part == "link_print")        translate([0, 0, X1 + lc[4]]) rotate([0, 90, 0]) link();   // flat outer face down, hooks up
if (part == "pins_print")        for (i = [0 : 3]) translate([i * 9, 0, 0]) pin_flat();
if (part == "fit_print") {       // one plate: bezel, both side strips, top and bottom strips, two pins
    flip(front_z) fit_bezel();
    translate([-40, 62, -X0]) rotate([0, -90, 0]) fit_side(true);
    translate([130, 62, X1]) rotate([0, 90, 0]) fit_side(false);
    translate([0, 30, -Zp]) fit_top();
    translate([0, -78, -Zp]) fit_bottom();
    for (i = [0, 1]) translate([100 + i * 9, -60, 0]) pin_flat();
}
if (part == "assembly_camera") {
    color("#1b1c1f") frame();
    color("#f2f2ee") chassis();
    color("#2b2c30") camera_back();
    color("#1b1c1f") lens_ring();
    color("#1b1c1f") link();
}
if (part == "assembly_plain") {
    color("#1b1c1f") frame();
    color("#f2f2ee") chassis();
    color("#2b2c30") plain_back();
}

// ---------------------------------------------------------------- interference checks (tools/check_fit.py)
chk_a = ""; chk_b = "";
module chk_sel(n) {
    if (n == "frame") frame();
    if (n == "chassis") chassis();
    if (n == "camera_back") camera_back();
    if (n == "plain_back") plain_back();
    if (n == "plunger") plunger();
    if (n == "pins") pins_in_place();
    if (n == "link") link();
    if (n == "lens_ring") lens_ring();
    if (n == "device") difference() {      // solid body (R3 corners, raised screen plate) minus its two top slots
        union() {
            linear_extrude(dev_t) rr2d(0, 0, dev_w, dev_h, dev_r);
            translate([raised[0], raised[1], dev_t - 0.01]) cube([raised[2] - raised[0], raised[3] - raised[1], 1.01]);
        }
        for (s = slots) translate([s[0] - slot_w / 2, 48.3, -1]) cube([slot_w, 10, slot_depth + 1]);
    }
    if (n == "uv") translate([k_x0, k_bot, k_back - k_size[3]]) {
        cube([k_size[0], k_size[1], k_size[3]]);                                                  // tail
        translate([0, k_size[1] - k_size[4], k_size[3] - k_size[2]]) cube([k_size[0], k_size[4], k_size[2]]);  // head
    }
}
if (part == "chk") intersection() { chk_sel(chk_a); chk_sel(chk_b); }
