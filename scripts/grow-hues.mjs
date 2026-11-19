// Grow Bezel palettes from hues all round the colour wheel with Detent's own
// generator, for "your own colour", and write them as a Rust table for the
// renderer. Every palette must pass Bezel's contrast checks, or this stops.
//
//   node scripts/grow-hues.mjs <detent>/cli/detent.mjs userland/crates/bezel/src/hues.rs
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const STEP = 5; // degrees between hues: 72 round the wheel
const [cli, out] = process.argv.slice(2);
if (!cli || !out) throw new Error("usage: grow-hues.mjs <detent.mjs> <hues.rs>");
const roles = ["ground", "surface1", "surface2", "surface3", "track", "meter", "ink", "dim", "faint", "signal", "onSignal", "ice", "onIce", "amber", "onAmber", "amberContainer", "amberInk"];

// A seed for a hue: strong but not neon, so every hue gives a usable signal.
function seed(hue) {
  const s = 0.72, v = 0.92;
  const c = v * s, x = c * (1 - Math.abs(((hue / 60) % 2) - 1)), m = v - c;
  const [r, g, b] = hue < 60 ? [c, x, 0] : hue < 120 ? [x, c, 0] : hue < 180 ? [0, c, x] : hue < 240 ? [0, x, c] : hue < 300 ? [x, 0, c] : [c, 0, x];
  return [r, g, b].map((v) => Math.round((v + m) * 255).toString(16).padStart(2, "0")).join("");
}

const rows = [];
for (let hue = 0; hue < 360; hue += STEP) {
  const hex = seed(hue);
  const tone = (t) => {
    const json = JSON.parse(execFileSync("node", [cli, "bezel", "--tone", t, "--seed", hex, "--json"], { encoding: "utf8" }));
    const failed = json.contrast.filter((c) => !c.pass).map((c) => c.use);
    if (failed.length) throw new Error(`#${hex} ${t}: ${failed.join(", ")}`);
    return "[" + roles.map((r) => "0x" + json.colors[r].slice(1)).join(", ") + "]";
  };
  rows.push(`    // hue ${hue}°, seed #${hex}\n    (0x${hex}, ${tone("dark")}, ${tone("light")}),`);
}
writeFileSync(out, `//! Palettes grown by Detent's \`bezel\` generator from ${rows.length} seeds round the
//! colour wheel, every ${STEP}°, for "your own colour". Each passes Bezel's own
//! contrast checks (the generator was asked, and refused none). Written by
//! \`scripts/grow-hues.mjs\`; the order of each row is that of \`palette\` in
//! \`color.rs\`.

/// Degrees between one hue and the next.
pub const STEP: u32 = ${STEP};

/// (seed, dark palette, light palette).
pub const HUES: [(u32, [u32; 17], [u32; 17]); ${rows.length}] = [
${rows.join("\n")}
];
`);
console.log("wrote", rows.length, "hues");
