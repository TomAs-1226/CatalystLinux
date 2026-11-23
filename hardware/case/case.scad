// Catalyst case for the M5Stack CardputerZero -- build version.
//
// One frame and one chassis take either of two backplates, swapped with a screwdriver:
//   camera backplate  UnitV K210 pod, gold camera trim, light-blue Grove link cover
//   plain backplate   slim protective back, IMX219/IR window, small gold accent
//
// Coordinates (mm), measured from M5's own enclosure STL (CONCEPT.md):
//   X right (0 = left side, seen from the screen), Y up (0 = keyboard edge,
//   54 = top edge with the EXT header), Z out of the screen (0 = device back,
//   22.1 = front face). The backplates sit at negative Z.
//
// Export:  openscad -D 'part="frame_print"' -o frame.stl case.scad   (tools/build.py does all)
// Parts are listed at the bottom; *_print variants are oriented for the printer bed.

part = "assembly_camera";
$fn = 64;

/* ===== 1. Device: measured (M5 STL, size drawing, docs) ===== */
dev_w = 84;            // drawing 84.0
dev_h = 54;            // drawing 54.0
dev_t = 22.1;          // front face (screen bezel rises to 23.1)
dev_r = 3.0;           // plan corner radius (STL sections)
imx   = [42.0, 47.8];  // IMX219 lens centre on the back
// The device's own M2 mounting holes, all on the back's top strip. They are not
// M5's shell screws: those are the two through screws at shell_screws, never used here.
m2_blind        = [[28, 50], [56, 50]];  // blind, beside the camera strip
m2_blind_depth  = 3.5;                   // measured in the STL
m2_pocket       = [[14, 50], [70, 50]];  // at the floor of 4.4 mm slots
m2_pocket_floor = 11.7;                  // slot depth from the back face
shell_screws    = [[13, 30], [70, 40]];  // M5's shell screws: left alone

/* ===== 2. Best estimates (not published by M5). Generous where unsure. ===== */
key_top    = 23.2;                     // keycap tops (drawing: 23.2 overall)
imx_fov    = [62.2, 48.8];             // assumed standard IMX219 lens, H x V degrees
fov_margin = 3;                        // degrees added on every side
ir_win     = [30.0, 49.0, 54.0, 52.0]; // the two windows beside the IMX219 (x0 y0 x1 y1), taken as IR
ir_shave   = 0.8;                      // outer IR-window edge left covered (screw heads sit there)
side_ctrl  = [40.2, 2.0, 46.0, 5.4];   // recessed control near the back, right side (y0 z0 y1 z1)
pwr_slide  = [8.6, 17.0, 14.4, 20.0];  // top slot, taken as the power slide (x0 z0 x1 z1)
ctrl_room  = 1.2;                      // extra room around controls of unconfirmed purpose
m2_engage  = 3.0;                      // thread engagement aimed for in the pocket holes
k_size     = [24, 40, 12.7, 8.2, 17];  // UnitV: width, length, head and tail thickness, head length

/* ===== 3. Fit: tune these from the fit test (PRINTING.md, "Fit test") ===== */
gap        = 0.25;     // device-to-frame clearance per side
lip_over   = 0.8;      // front lip overlap onto the device face
port_room  = 0.0;      // grows every port window, per side
hole_shift = [0, 0];   // moves the whole M2 hole pattern
m2_clear   = 2.4;      // M2 clearance hole
snap       = 0.35;     // backplate snap-bump height into the frame
nut_af     = 4.15;     // M2 hex nut pocket (across flats)

/* ===== 4. Screws: standard lengths; seats are derived from them ===== */
L_chassis       = 4;   // M2 x 4 countersunk: chassis into the blind holes
L_pocket_plain  = 16;  // M2 x 16 button head: plain backplate into the pocket holes
L_pocket_camera = 25;  // M2 x 25 socket head: camera backplate into the X14 pocket hole
L_nut_plain     = 3;   // M2 x 3 countersunk: plain backplate into the chassis nuts
L_nut_camera    = 6;   // M2 x 6 button head: camera backplate into the chassis nuts
nut_pos         = [[6, 6], [79, 6]];   // captive M2 nuts in the chassis
cb_d            = 4.2; // counterbore for M2 button / socket heads

/* ===== 5. Case ===== */
wall       = 2.0;   // frame side wall
lip_t      = 1.6;   // front lip thickness
chassis_t  = 1.6;   // PETG chassis plate
skin       = 1.6;   // camera backplate skin
plain_t    = 1.8;   // plain backplate thickness
lip_w      = 1.0;   // backplate locating lip inside the frame's rear ring
front_edge = 1.4;   // radius on the frame's front edge
edge_top   = 2.4;   // camera backplate rear-edge radius at the camera end
edge_bot   = 5.0;   // ... at the grip end (<= R_out)
plain_edge = 1.2;   // plain backplate rear-edge radius
k_x1       = 80.8;  // UnitV outer long side (leaves a 3.85 mm cable lane to the wall)
k_top      = 52.0;  // UnitV lens end

// ---------------------------------------------------------------- derived
R_cav   = dev_r + gap;               // 3.25
R_out   = R_cav + wall;              // 5.25 (concentric: outer = inner + wall)
X0 = -gap - wall;  X1 = dev_w + gap + wall;
Y0 = -gap - wall;  Y1 = dev_h + gap + wall;
lip_z0  = dev_t + 0.2;
front_z = lip_z0 + lip_t;            // 23.9
c_in    = -gap + 0.1 + lip_w + 0.1;  // chassis inset (0.95): room for the backplate lip
k_x0    = k_x1 - k_size[0];
k_bot   = k_top - k_size[1];
lens    = [k_x1 - 12, k_top - 9];    // M12 lens centre
pod     = chassis_t + k_size[2] + 0.3 + skin;  // 16.2: set by the UnitV's lens end
Zc      = -pod;                      // camera backplate back surface
Zp      = -(chassis_t + plain_t);    // plain backplate back surface (-3.4)
mh      = [for (p = concat(m2_blind, m2_pocket)) p + hole_shift];
pocket  = [for (p = m2_pocket) p + hole_shift];
blind   = [for (p = m2_blind) p + hole_shift];
function fov_x(d) = 1.5 + d * tan(imx_fov[0] / 2 + fov_margin);
function fov_y(d) = 1.5 + d * tan(imx_fov[1] / 2 + fov_margin);
// camera opening at the chassis: the IR windows (shaved) and the IMX219 cone
win = [ir_win[0] + ir_shave, min(ir_win[1], imx[1] - fov_y(chassis_t)) - 0.4, ir_win[2] - ir_shave];
// camera-backplate bay at its back surface (open to the top edge)
bay = [imx[0] - fov_x(pod) - 1.0, imx[1] - fov_y(pod) - 1.2, imx[0] + fov_x(pod) + 0.6];
seat_pocket_camera = m2_pocket_floor + m2_engage - L_pocket_camera;  // head seat Z
seat_nut_camera    = -0.3 - L_nut_camera;

echo(str("thickness camera ", front_z - Zc, " mm (ring +2.4), plain ", front_z - Zp, " mm"));
echo(str("pocket-hole engagement: plain ", Zp + 1.2 + L_pocket_plain - m2_pocket_floor,
         " mm, camera ", m2_engage, " mm"));
assert(front_z >= key_top + 0.5, "front lip must stand proud of the keys");
assert(bay[2] + 1.2 <= k_x0 - 0.2, "bay wall would hit the UnitV");

// ---------------------------------------------------------------- helpers
module r2(a0, b0, a1, b1) translate([a0, b0]) square([a1 - a0, b1 - b0]);
module rr2d(x0, y0, x1, y1, r)
    translate([x0 + r, y0 + r]) offset(r = r) square([x1 - x0 - 2 * r, y1 - y0 - 2 * r]);
module round2d(r) offset(r = r) offset(delta = -r) offset(r = -r) offset(delta = r) children();
module capsule2d(a0, b0, a1, b1) rr2d(a0, b0, a1, b1, min(a1 - a0, b1 - b0) / 2 - 0.001);
// A disc of radius R, height h, rounded bottom (eb) and top (et) edges. cham turns the
// first part of the bottom round into a 45 deg chamfer so it prints without droop.
module puck(R, h, eb, et, cham = false)
    rotate_extrude() intersection() {
        square([R, h]);
        hull() {
            square([0.01, h]);
            if (eb > 0) translate([R - eb, eb]) circle(eb); else square([R, 0.01]);
            if (eb > 0 && cham) translate([R - 0.586 * eb, 0]) square(0.01);
            if (et > 0) translate([R - et, h - et]) circle(et); else translate([0, h - 0.01]) square([R, 0.01]);
        }
    }
module softbox2(x0, y0, x1, y1, z0, z1, R, eb_bot, eb_top, et, cham = false)
    hull() for (x = [x0 + R, x1 - R]) {
        translate([x, y0 + R, z0]) puck(R, z1 - z0, eb_bot, et, cham);
        translate([x, y1 - R, z0]) puck(R, z1 - z0, eb_top, et, cham);
    }
module ext_x(x, len) translate([x, 0, 0]) rotate([90, 0, 90]) linear_extrude(len) children(); // 2D(Y,Z) -> +X
module ext_y(y, len) translate([0, y, 0]) rotate([90, 0, 0]) linear_extrude(len) children();  // 2D(X,Z) -> -Y
module grow() offset(delta = port_room) children();

// ---------------------------------------------------------------- frame (black PLA)
module left_windows()   // RJ45, USB switch, USB-A, USB-C1 -- plug overmolds included
    grow() round2d(1.6) union() {
        r2(4.2, 1.4, 22.0, 17.6);     // RJ45 (jack Y5.0-21.2 Z2.2-16.8) + boot
        r2(21.0, 12.6, 33.6, 17.6);   // bridge
        r2(23.2, 12.6, 31.8, 21.0);   // USB switch (Y24.0-31.0 Z13.4-20.2)
        r2(32.6, 8.0, 47.4, 21.6);    // USB-A (Y33.4-46.6 Z8.8-16.0) + USB-C1 (Y35.6-44.4 Z17-20.2)
    }
// HDMI type A (Y5.8-20.6 Z9.8-15.4), TRRS (Y25-30 Z11.4-16.8), Grove (Y34-46 Z10.6-17), USB-C2 (Y35.5-44.5 Z17-20.2)
module right_windows() grow() rr2d(3.0, 7.0, 47.4, 21.6, 2.5);
module top_windows()    // power slide, microSD, EXT 2x7, slot, vents
    grow() round2d(1.2) union() {
        r2(pwr_slide[0] - 2.2 - ctrl_room, 15.0, 31.4, 21.4);  // power slide + microSD (X18.2-29.8 Z17-18.8)
        r2(31.4, 6.6, 52.6, 19.9);    // EXT header (X32.8-51.2 Z10.4-16.8) + slot (X37.8-46 Z18.6-19.6)
        r2(20.2, 6.6, 31.4, 10.4);    // vents X20.8-30.2
        r2(52.6, 6.6, 63.8, 10.4);    // vents X53.8-63.2
    }
snap_x = [18, 66];
module frame() {
    o  = lip_over;
    Ro = R_out - (wall + gap + o);   // concentric lip opening radius
    difference() {
        softbox2(X0, Y0, X1, Y1, -chassis_t, front_z, R_out, 0.3, 0.3, front_edge);
        translate([0, 0, -chassis_t - 1]) linear_extrude(lip_z0 + chassis_t + 1)
            rr2d(-gap, -gap, dev_w + gap, dev_h + gap, R_cav);
        translate([0, 0, lip_z0 - 1]) linear_extrude(front_z - lip_z0 + 0.2) rr2d(o, o, dev_w - o, dev_h - o, Ro);
        hull() {   // 0.8 mm thumb chamfer
            translate([0, 0, front_z - 0.8]) linear_extrude(0.01) rr2d(o, o, dev_w - o, dev_h - o, Ro);
            translate([0, 0, front_z + 0.01]) linear_extrude(0.01) rr2d(o - 0.8, o - 0.8, dev_w - o + 0.8, dev_h - o + 0.8, Ro + 0.8);
        }
        ext_x(X0 - 1, wall + gap + 1.5) left_windows();
        ext_x(dev_w - 0.5, wall + gap + 1.5) {
            right_windows();
            capsule2d(13.2, 0.4, 21.8, 5.0);   // three slots at the back-right corner
            grow() capsule2d(side_ctrl[0] - ctrl_room, side_ctrl[1] - ctrl_room,
                             side_ctrl[2] + ctrl_room, side_ctrl[3] + ctrl_room);
        }
        ext_y(Y1 + 1, wall + gap + 1.5) { top_windows(); grow() capsule2d(74.6, 16.4, 77.4, 19.6); } // + pinhole
        ext_y(0.5, wall + gap + 1.5) grow() capsule2d(29.6, 10.0, 54.4, 16.4);                       // speaker
        // dimples for the backplate's snap bumps
        for (x = snap_x, y = [-gap, dev_h + gap]) translate([x, y, -0.9]) sphere(r = 0.75, $fn = 24);
    }
}

// ---------------------------------------------------------------- chassis (white PETG)
module chassis() {
    difference() {
        union() {
            translate([0, 0, -chassis_t]) linear_extrude(chassis_t)
                rr2d(c_in, c_in, dev_w - c_in, dev_h - c_in, R_cav - gap - c_in);
            for (p = nut_pos) translate([p[0], p[1], -chassis_t - 0.6]) cylinder(d = 7, h = 0.61);
        }
        // camera opening (IMX219 cone + IR windows), open to the top edge
        translate([0, 0, -chassis_t - 1]) linear_extrude(chassis_t + 2) rr2d(win[0], win[1], win[2], 60, 2.2);
        // M2 x 4 countersunk into the blind holes (flush, under either backplate)
        for (p = blind) translate([p[0], p[1], -chassis_t - 1]) {
            cylinder(d = m2_clear, h = 5);
            translate([0, 0, 1]) cylinder(d1 = 4.2, d2 = m2_clear, h = 0.9);
        }
        // clearance for the backplate screws into the pocket holes
        for (p = pocket) translate([p[0], p[1], -chassis_t - 1]) cylinder(d = m2_clear, h = 5);
        // captive M2 nuts, dropped in from the device side
        for (p = nut_pos) translate([p[0], p[1], 0]) {
            translate([0, 0, -1.65]) cylinder(d = nut_af / cos(30), h = 2, $fn = 6);
            translate([0, 0, -3]) cylinder(d = m2_clear, h = 4);
        }
        // keep the device's back slit (X10.8-23.2, Y25) breathing
        translate([0, 0, -chassis_t - 1]) linear_extrude(chassis_t + 2) rr2d(10.4, 24.0, 23.6, 26.0, 0.9);
    }
}

// ---------------------------------------------------------------- shared backplate pieces
module lip_ring(z1 = -0.2)   // locating lip inside the frame's rear ring, with snap bumps
    difference() {
        union() {
            translate([0, 0, -chassis_t - 0.01]) linear_extrude(z1 + chassis_t + 0.01)
                difference() {
                    rr2d(-gap + 0.1, -gap + 0.1, dev_w + gap - 0.1, dev_h + gap - 0.1, R_cav - 0.1);
                    rr2d(-gap + 0.1 + lip_w, -gap + 0.1 + lip_w, dev_w + gap - 0.1 - lip_w, dev_h + gap - 0.1 - lip_w, R_cav - 0.1 - lip_w);
                }
            for (x = snap_x) {
                translate([x, -gap + 0.15, -0.9]) scale([1, (snap + 0.05) / 0.6, 1]) sphere(r = 0.6, $fn = 24);
                translate([x, dev_h + gap - 0.15, -0.9]) scale([1, (snap + 0.05) / 0.6, 1]) sphere(r = 0.6, $fn = 24);
            }
        }
        // flex slots either side of each bump so the lip can give
        for (x = snap_x, y = [-gap + 0.1 + lip_w / 2, dev_h + gap - 0.1 - lip_w / 2], dx = [-3.5, 3.5])
            translate([x + dx - 0.3, y - 1, -0.9]) cube([0.6, 2, 1]);
    }

// ---------------------------------------------------------------- camera backplate (black PLA)
module bay_cut() {
    hull() {
        translate([0, 0, -chassis_t + 0.01]) linear_extrude(0.01) rr2d(win[0], win[1], win[2], 66, 2.2);
        translate([0, 0, Zc - 0.01]) linear_extrude(0.01) rr2d(bay[0], bay[1], bay[2], 66, 4.0);
    }
    translate([0, 0, Zc - 6]) linear_extrude(6) rr2d(bay[0], bay[1], bay[2], 66, 4.0);
}
module bay_sleeve()
    hull() {
        translate([0, 0, -chassis_t - 0.01]) linear_extrude(0.01) rr2d(win[0] - skin, win[1] - skin, win[2] + skin, Y1 - skin, 2.2 + skin);
        translate([0, 0, Zc + 0.5]) linear_extrude(0.01) rr2d(bay[0] - skin, bay[1] - skin, bay[2] + 1.2, Y1 - skin, 4.0 + skin);
    }
module trim2d()    // U around the bay + ring around the K210 lens
    round2d(1.0) union() {
        difference() {
            rr2d(bay[0] - 2.4, bay[1] - 2.4, bay[2] + 2.4, Y1 - edge_top, 4 + 2.4);
            rr2d(bay[0], bay[1], bay[2], 70, 4);
        }
        translate(lens) circle(d = 21.6);
        translate([bay[2] + 1, lens[1] - 5]) square([lens[0] - 10.8 - bay[2] + 1.5, 10]);
    }
module cam_shell()
    softbox2(X0, Y0, X1, Y1, Zc, -chassis_t, R_out, edge_bot, edge_top, 0.3, cham = true);
k_tail_z = -chassis_t - k_size[3];   // UnitV tail front face (-9.8)
module k210_seat()   // pins into the UnitV's LEGO holes, pads that hold its tail down
    for (x = [k_x1 - 4, k_x1 - 20]) translate([x, k_top - 28.2, Zc + skin - 0.01]) {
        cylinder(d = 7, h = (k_tail_z - 0.2) - (Zc + skin) + 0.01);
        cylinder(d = 4.5, h = (k_tail_z + 1.5) - (Zc + skin) + 0.01);
    }
module camera_back() {
    difference() {
        union() {
            difference() {
                cam_shell();
                softbox2(X0 + skin, Y0 + skin, X1 - skin, Y1 - skin, Zc + skin, -chassis_t + 1, R_out - skin,
                         edge_bot - skin, edge_top - skin, 0);
            }
            intersection() { bay_sleeve(); cam_shell(); }
            lip_ring();
            k210_seat();
            // screw columns (P14 and the two chassis nuts)
            for (p = [pocket[0], nut_pos[0], nut_pos[1]])
                translate([p[0], p[1], Zc + 0.5]) cylinder(d = 6.4, h = -Zc - chassis_t - 0.5 - (p == pocket[0] ? 0 : 0.6));
            // guide for the button-B plunger
            translate([k_x0 - 1.4, 17.8, Zc + skin]) difference() {   // open toward the UnitV
                translate([-2.0, -2.6, 0]) cube([3.0, 5.2, 9]);
                translate([-1.15, -1.65, -1]) cube([2.3, 3.3, 11]);
            }
            // rib for the link cover's screw
            translate([k_x1 + 0.4, 42, -10]) cube([X1 - skin - k_x1 - 0.39, 4, 8]);
        }
        bay_cut();
        translate([0, 0, Zc - 1]) linear_extrude(1 + 0.8) trim2d();                 // trim pocket
        translate([lens[0], lens[1], Zc - 1]) cylinder(d = 18.0, h = skin + 3);     // M12 lens well
        translate([0, 0, Zc - 1]) linear_extrude(1.5) rr2d(7.5, 9.5, 76.5, 27.5, 4.0);  // grip panel, 0.5 step
        // UnitV microSD (its lens end) through the top face, with a fingernail scoop
        hull() {
            ext_y(Y1 - skin + 0.01, 0.01) rr2d(k_x1 - 17.4, -9.0, k_x1 - 4.4, -4.0, 2.0);
            ext_y(Y1 + 0.01, 0.01) rr2d(k_x1 - 18.4, -10.0, k_x1 - 3.4, -3.0, 2.4);
        }
        ext_y(Y1 + 1, skin + 3) rr2d(k_x1 - 17.4, -9.0, k_x1 - 4.4, -4.0, 2.0);
        // UnitV USB-C tunnel to the bottom face
        ext_y(k_bot + 0.5, 20) rr2d(k_x1 - 19, -10.4, k_x1 - 5, -1.9, 3.0);
        // button A: flexure key in the right face (U slot); its nub is added below
        ext_x(X1 - skin - 1, skin + 2) difference() {
            rr2d(k_top - 38.6, -9.8, k_top - 30.0, -2.2, 2.2);
            rr2d(k_top - 37.8, -9.0, k_top - 30.8, -3.0, 1.6);
            translate([k_top - 37.8, -9.8]) square([7.0, 0.8]);   // hinge along the bottom
        }
        // button B: pin key (a printed wedge plunger turns the push sideways)
        translate([k_x0 - 1.4, 17.8, Zc - 1]) cylinder(d = 2.3, h = skin + 2, $fn = 24);
        // screws: P14 (socket head in a bore), chassis nuts (button heads in bores)
        translate([pocket[0][0], pocket[0][1], 0]) {
            translate([0, 0, Zc - 1]) cylinder(d = cb_d, h = seat_pocket_camera - Zc + 1);
            translate([0, 0, Zc - 1]) cylinder(d = m2_clear, h = 30);
        }
        for (p = nut_pos) translate([p[0], p[1], 0]) {
            translate([0, 0, Zc - 1]) cylinder(d = cb_d, h = seat_nut_camera - Zc + 1);
            translate([0, 0, Zc - 1]) cylinder(d = m2_clear, h = 30);
            translate([0, 0, -chassis_t - 0.62]) cylinder(d = 7.4, h = 1);   // clears the nut boss
        }
        // Grove cable exit into the link cover
        ext_x(X1 - skin - 1, skin + 2) rr2d(33.8, -11.2, 37.6, -2.6, 1.6);
        // link-cover screw (M2 x 6 self-tapping into the rib)
        translate([X1 - 8, 44, -6.3]) rotate([0, 90, 0]) cylinder(d = 1.7, h = 10);
        // fingernail notch to lift the trim for a lens swap
        translate([lens[0], lens[1] - 10.8 - 0.9, Zc - 1]) cylinder(d = 3.0, h = 1 + 0.8, $fn = 24);
    }
    // button A nub on the flexure tab
    translate([k_x1 + 0.4, k_top - 34.2 - 1.5, -chassis_t - 4.3 - 1.5]) cube([X1 - skin - k_x1 - 0.39, 3, 3]);
}

// ---------------------------------------------------------------- plain backplate (black PLA)
plain_win = [win[0], imx[1] - fov_y(-Zp) - 0.6, win[2], ir_win[3] + 0.6];
accent    = [35.0, 40.4, 49.0, 42.4];   // gold capsule under the window
module plain_back() {
    difference() {
        union() {
            softbox2(X0, Y0, X1, Y1, Zp, -chassis_t, R_out, plain_edge, plain_edge, 0.3, cham = true);
            lip_ring();
        }
        translate([0, 0, Zp - 1]) linear_extrude(plain_t + 2) rr2d(plain_win[0], plain_win[1], plain_win[2], plain_win[3], 2.2);
        translate([0, 0, Zp - 1]) linear_extrude(1 + 0.8) offset(0.1) capsule2d(accent[0], accent[1], accent[2], accent[3]);
        translate([0, 0, Zp - 1]) linear_extrude(1.4) rr2d(7.5, 9.5, 76.5, 27.5, 4.0);     // grip panel, 0.4 step
        translate([0, 0, Zp - 1]) linear_extrude(plain_t + 2) rr2d(10.6, 24.4, 23.4, 25.6, 0.6);  // vent over the slit
        for (p = pocket) translate([p[0], p[1], Zp - 1]) { cylinder(d = m2_clear, h = 10); cylinder(d = cb_d, h = 1 + 1.2); }
        for (p = nut_pos) translate([p[0], p[1], Zp - 0.01]) {
            cylinder(d = m2_clear, h = 10);
            cylinder(d1 = 4.2, d2 = m2_clear, h = 0.9);                              // countersink
            translate([0, 0, plain_t - 0.62]) cylinder(d = 7.4, h = 1);              // clears the nut boss
        }
    }
}

// ---------------------------------------------------------------- small parts
module trim() {    // gold PLA: inlay + raised lens ring
    difference() {
        union() {
            translate([0, 0, Zc]) linear_extrude(0.8) difference() { trim2d(); translate(lens) circle(d = 18.0); }
            translate([lens[0], lens[1], Zc - 2.4]) difference() {
                cylinder(d = 21.6, h = 2.4 + 0.01);
                translate([0, 0, -1]) cylinder(d = 18.0, h = 5);
                translate([0, 0, -0.01]) cylinder(d1 = 19.4, d2 = 18.0, h = 0.7);
            }
        }
    }
}
module accent() translate([0, 0, Zp]) linear_extrude(0.8) capsule2d(accent[0], accent[1], accent[2], accent[3]);
module soft_bar(y0, y1, z0, z1, p, r = 2.4)
    hull() for (y = [y0 + r, y1 - r], z = [z0 + r, z1 - r]) {
        translate([X1 + p - r, y, z]) sphere(r = r, $fn = 32);
        translate([X1, y, z]) rotate([0, 90, 0]) cylinder(r = r, h = 0.01, $fn = 32);
    }
// The Grove socket sits right under USB-C2, so the cover stops below the plug:
// its top face carries the plug, a channel takes the cable back into the pod,
// and a window keeps the recessed side control reachable.
module link() {    // light-blue PLA
    difference() {
        soft_bar(32.6, 47.4, -11.0, 10.6, 4.6);
        translate([X1 - 50, -50, -50]) cube([50, 200, 200]);               // flat inner face
        translate([X1 - 1, 33.8, -11.6]) cube([3.6, 3.8, 24]);              // cable channel
        hull() {   // window onto the side control, chamfered for a fingertip
            translate([X1 - 1, 0, 0]) ext_x(0, 0.01) capsule2d(side_ctrl[0] - 0.6, side_ctrl[1] - 1.0, side_ctrl[2] + 0.6, side_ctrl[3] + 1.0);
            translate([X1 + 4.61, 0, 0]) ext_x(0, 0.01) capsule2d(side_ctrl[0] - 1.8, side_ctrl[1] - 2.2, side_ctrl[2] + 1.8, side_ctrl[3] + 2.2);
        }
        translate([X1 - 1, 44, -6.3]) rotate([0, 90, 0]) { cylinder(d = m2_clear, h = 10); translate([0, 0, 3.4]) cylinder(d = cb_d, h = 5); }
    }
}
module plunger() {  // white PETG: pin + wedge for the UnitV's inner button
    translate([k_x0 - 1.4, 17.8, 0]) {
        translate([0, 0, Zc - 0.4]) cylinder(d = 2.0, h = skin + 0.6, $fn = 24);                       // pin
        translate([-1.0, -1.5, Zc + skin]) cube([2.0, 3.0, 9.0]);                                       // stem
        hull() {   // wedge: as it moves +Z it pushes the side button toward +X
            translate([-1.0, -1.5, Zc + skin + 9.0]) cube([2.0, 3.0, 0.01]);
            translate([-1.0, -1.5, Zc + skin + 11.0]) cube([2.3, 3.0, 0.01]);   // tune after fitting
        }
    }
}

// ---------------------------------------------------------------- fit test (black PLA, about 25 min)
module fit_ring()     // the front 6 mm of the frame: cavity, corners, lip, key and screen clearance
    intersection() { frame(); translate([X0 - 1, Y0 - 1, lip_z0 - 4.4]) cube([X1 - X0 + 2, Y1 - Y0 + 2, 10]); }
module fit_side(left) // a full-height side wall with its port windows
    intersection() {
        frame();
        translate([left ? X0 - 1 : dev_w - 1.2, Y0 - 1, -chassis_t - 1]) cube([2.2 + 1.2 + gap, Y1 - Y0 + 2, 30]);
    }
module fit_holes()    // the backplate's M2 pattern + camera window, hooks over the top edge
    difference() {
        union() {
            translate([0, 0, -1.2]) linear_extrude(1.2) rr2d(4, 44, 80, dev_h + gap, 1.5);
            translate([4, dev_h + gap, -1.2]) cube([76, 1.2, 4.2]);   // hooks over the device's top edge
        }
        translate([0, 0, -2]) linear_extrude(3) rr2d(win[0], win[1], win[2], 60, 2.2);
        for (p = mh) translate([p[0], p[1], -2]) cylinder(d = 2.2, h = 3, $fn = 24);
    }

// ---------------------------------------------------------------- render-only dummies
module key() hull() for (dx = [-1.0, 1.0]) translate([dx, 0, 0]) cylinder(d = 2.3, h = 1.55, $fn = 24);
module keys_all() {
    for (i = [0 : 10], y = [3.5, 10.0, 16.5, 23.0]) translate([4.5 + 7.5 * i, y, dev_t - 0.6]) key();
    for (x = [4.5, 79.5]) translate([x, 29.5, dev_t - 0.6]) key();
}
module lenses() translate([lens[0], lens[1], -chassis_t - k_size[2] - 3.7]) difference() {
    cylinder(d = 14, h = 3.7);
    translate([0, 0, -0.01]) cylinder(d = 8.6, h = 0.5);
}
module glass_imx() translate([imx[0], imx[1], -0.5]) cylinder(d = 5.2, h = 0.5);
module glass_k210() translate([lens[0], lens[1], -chassis_t - k_size[2] - 3.7 - 0.3]) cylinder(d = 8.4, h = 0.8);
module screws(variant) {   // button / socket heads, render only
    pts = variant == "camera"
        ? [[pocket[0], seat_pocket_camera], [nut_pos[0], seat_nut_camera], [nut_pos[1], seat_nut_camera]]
        : [[pocket[0], Zp + 1.2], [pocket[1], Zp + 1.2], [nut_pos[0], Zp], [nut_pos[1], Zp]];
    for (q = pts) translate([q[0][0], q[0][1], q[1] - 1.1]) difference() {
        cylinder(d = 3.6, h = 1.1, $fn = 32);
        translate([0, 0, -0.01]) cylinder(d = 1.4, h = 0.6, $fn = 6);
    }
}

// ---------------------------------------------------------------- print orientation
module on_bed(z) translate([0, 0, -z]) children();
module flip(ztop) translate([0, 0, ztop]) rotate([180, 0, 0]) children();

// ---------------------------------------------------------------- output
if (part == "frame")          frame();
if (part == "chassis")        chassis();
if (part == "camera_back")    camera_back();
if (part == "plain_back")     plain_back();
if (part == "trim")           trim();
if (part == "accent")         accent();
if (part == "link")           link();
if (part == "plunger")        plunger();
if (part == "keys")           keys_all();
if (part == "lenses")         lenses();
if (part == "glass_imx")      glass_imx();
if (part == "glass_k210")     glass_k210();
if (part == "screws_camera")  screws("camera");
if (part == "screws_plain")   screws("plain");
// oriented for the bed (z = 0 is the bed)
if (part == "frame_print")       flip(front_z) frame();                         // lip face down
if (part == "chassis_print")     flip(0) chassis();                             // device side down
if (part == "camera_back_print") on_bed(Zc) camera_back();                      // back face down
if (part == "plain_back_print")  on_bed(Zp) plain_back();                       // back face down
if (part == "trim_print")        flip(Zc + 0.8) trim();                         // pocket side down
if (part == "accent_print")      flip(Zp + 0.8) accent();
if (part == "link_print")        translate([0, 0, -X1]) rotate([0, -90, 0]) link();  // inner face down
if (part == "plunger_print")     translate([0, 0, -(k_x0 - 2.4)]) rotate([0, -90, 0]) plunger();  // flat side down
if (part == "fit_print") {       // one plate: ring, both side strips, hole strip
    translate([0, 0, 0]) flip(front_z) fit_ring();
    translate([-35, 60, -X0]) rotate([0, -90, 0]) fit_side(true);
    translate([130, 60, X1]) rotate([0, 90, 0]) fit_side(false);
    translate([0, 76, 1.2]) fit_holes();
}
if (part == "assembly_camera") {
    color("#1b1c1f") frame();
    color("#f2f2ee") chassis();
    color("#1b1c1f") camera_back();
    color("#c9a25a") trim();
    color("#9cc6e6") link();
}
if (part == "assembly_plain") {
    color("#1b1c1f") frame();
    color("#f2f2ee") chassis();
    color("#1b1c1f") plain_back();
    color("#c9a25a") accent();
}
