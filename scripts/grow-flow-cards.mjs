// Grow the colours of Flow's album cards (Home's third style) with Detent's
// own generator: for each of the 72 hues "your own colour" offers, the two
// ends of Bezel's case gradient (the seed at signal chroma, tones 64 and 52),
// its signal and its dark ground, from the same seeds as grow-hues.mjs.
// Written as a Rust table for the shell.
//
//   node scripts/grow-flow-cards.mjs <detent>/cli/detent.mjs userland/apps/shell/src/flow/palette.rs
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";

const STEP = 5;
const [cli, out] = process.argv.slice(2);
if (!cli || !out) throw new Error("usage: grow-flow-cards.mjs <detent.mjs> <palette.rs>");

// The seed for a hue, exactly as grow-hues.mjs makes it.
function seed(hue) {
  const s = 0.72, v = 0.92;
  const c = v * s, x = c * (1 - Math.abs(((hue / 60) % 2) - 1)), m = v - c;
  const [r, g, b] = hue < 60 ? [c, x, 0] : hue < 120 ? [x, c, 0] : hue < 180 ? [0, c, x] : hue < 240 ? [0, x, c] : hue < 300 ? [x, 0, c] : [c, 0, x];
  return [r, g, b].map((v) => Math.round((v + m) * 255).toString(16).padStart(2, "0")).join("");
}

const rows = [];
for (let hue = 0; hue < 360; hue += STEP) {
  const hex = seed(hue);
  const json = JSON.parse(execFileSync("node", [cli, "bezel", "--tone", "dark", "--seed", hex, "--json"], { encoding: "utf8" }));
  const failed = json.contrast.filter((c) => !c.pass).map((c) => c.use);
  if (failed.length) throw new Error(`#${hex}: ${failed.join(", ")}`);
  const c = json.colors;
  const h = (v) => "0x" + v.slice(1);
  rows.push(`    // hue ${hue}°, seed #${hex}\n    (${h(c.caseA)}, ${h(c.caseB)}, ${h(c.signal)}, ${h(c.ground)}),`);
}
writeFileSync(out, `//! The colours of Flow's album cards, grown by Detent's \`bezel\` generator
//! from the same ${rows.length} seeds as "your own colour" (\`bezel::hues\`): for each,
//! the two ends of Bezel's case gradient (the seed at signal chroma, tones 64
//! and 52), its signal, and its dark ground. Written by
//! \`scripts/grow-flow-cards.mjs\`.

/// (case gradient start, case gradient end, signal, ground), every ${STEP}°.
pub const CARDS: [(u32, u32, u32, u32); ${rows.length}] = [
${rows.join("\n")}
];
`);
console.log(`${rows.length} hues written to ${out}`);
