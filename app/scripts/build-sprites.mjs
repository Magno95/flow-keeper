// Generates the companion's clean frames from the ImageGen sheets.
//
//   node scripts/build-sprites.mjs "<sheets folder>" [previews folder]
//
// The sheets are concepts: dark background with a halo, casing and scale that
// vary slightly between frames. Here a single canonical casing is used (cleaned
// up once, with real transparency) and for each frame only the screen content
// (eyes, mouth, eyebrows) is carried over, realigned.

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import {
  bbox, distanceFrom, erode, floodFromBorder, isIvory, largestComponent, load, lum, newPng, px, resizeArea, save,
} from "./sprite-lib.mjs";

const src = process.argv[2];
const previewDir = process.argv[3];
const outDir = path.resolve("src/assets/companion");
fs.mkdirSync(outDir, { recursive: true });

const OUT_W = 192;
const SCREEN_INSET = 14; // excludes the screen's dark inner border
const brightTurq = ([r, g, b]) => g > 170 && b > 160 && r < 130;

// Sequences from the README: timings of frames 1–6 in milliseconds.
const SHEETS = {
  rest: { file: "01-rest", durations: [1800, 90, 110, 90, 300, 1600] },
  working: { file: "02-working", durations: [450, 120, 400, 250, 120, 400] },
  waiting: { file: "03-waiting", durations: [150, 120, 250, 160, 250, 1000] },
  problem: { file: "04-connection", durations: [150, 200, 250, 300, 300, 1000] },
  finished: { file: "05-turn-finished", durations: [100, 150, 300, 200, 150, 200] },
  wink: { file: "06-wink", durations: [80, 90, 180, 100, 90, 120] },
  down: { file: "07-look-down", durations: null },
  up: { file: "08-look-up", durations: null },
  genie: { file: "09-working-genie", durations: [300, 140, 700, 450, 450, 1400], accents: true },
  intense: { file: "10-working-intense", durations: [250, 200, 450, 700, 600, 1600] },
  new: { file: "11-new-chat", durations: [120, 110, 380, 260, 160, 220], accents: true, headroom: 0.26 },
  update: { file: "12-update-available", durations: [400, 160, 650, 450, 350, 900], bubble: true },
};

// Bubbles from the update sheet, as separate layers above the robot:
// the face stays free to show the current state. Index of the frame each
// bubble is taken from.
const BUBBLES = { "bubble-small": 1, bubble: 5 };

// Gold or orange elements (glasses, exclamation mark) that extend beyond the
// screen: they're carried over onto the canonical casing at the same proportion.
const isAccent = ([r, g, b]) => r > 190 && g > 130 && r - b >= 90;

/**
 * The six casings of a sheet, ordered by row. Bubbles are ivory like the
 * casing but much wider than tall: with `bubbles` they're collected there.
 */
function heads(img, accents = false, bubbles = null) {
  const { width: w, height: h } = img;
  const ivory = new Uint8Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const p = px(img, x, y);
    // Accents count as casing, so the glasses' temples don't split it.
    ivory[y * w + x] = isIvory(p) || (accents && isAccent(p)) ? 1 : 0;
  }
  const found = [];
  const taken = new Uint8Array(w * h);
  // With bubbles, keep going: the small ones come after the heads.
  for (let n = 0; n < (bubbles ? 12 : 6); n++) {
    const mask = new Uint8Array(w * h);
    for (let i = 0; i < w * h; i++) mask[i] = ivory[i] && !taken[i] ? 1 : 0;
    const comp = largestComponent(w, h, mask);
    for (let i = 0; i < w * h; i++) if (comp[i]) taken[i] = 1;
    const box = bbox(w, h, comp);
    if (bubbles && box.w > box.h * 1.4) bubbles.push({ mask: comp, box });
    else if (found.length < 6) found.push({ mask: comp, box });
  }
  found.sort((a, b) => (Math.abs(a.box.y0 - b.box.y0) > 100 ? a.box.y0 - b.box.y0 : a.box.x0 - b.box.x0));
  return found.map((f) => ({ ...f, screen: screenOf(img, f) }));
}

/** Dark region enclosed by the casing. */
function screenOf(img, head) {
  const { width: w } = img;
  const b = head.box;
  const local = (x, y) => head.mask[(b.y0 + y) * w + (b.x0 + x)];
  const outside = floodFromBorder(b.w, b.h, (x, y) => !local(x, y));
  const inner = new Uint8Array(b.w * b.h);
  for (let i = 0; i < b.w * b.h; i++) inner[i] = !outside[i] && !local(i % b.w, (i / b.w) | 0) ? 1 : 0;
  const comp = largestComponent(b.w, b.h, inner);
  const sb = bbox(b.w, b.h, comp);
  return { mask: comp, box: { x0: b.x0 + sb.x0, y0: b.y0 + sb.y0, w: sb.w, h: sb.h }, origin: { x: b.x0, y: b.y0, w: b.w } };
}

/** Canonical casing with the background removed: RGBA image of the box, and screen. */
function canonical(img, head) {
  const b = head.box;
  const region = { x0: Math.max(0, b.x0 - 80), y0: Math.max(0, b.y0 - 24), x1: Math.min(img.width - 1, b.x1 + 80), y1: Math.min(img.height - 1, b.y1 + 24) };
  const W = region.x1 - region.x0 + 1;
  const H = region.y1 - region.y0 + 1;
  const at = (x, y) => px(img, region.x0 + x, region.y0 + y);
  const solid = new Uint8Array(W * H);
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
    const p = at(x, y);
    solid[y * W + x] = isIvory(p) || brightTurq(p) ? 1 : 0;
  }
  // The black outline near the casing stops the fill; the halo doesn't.
  const near = distanceFrom(W, H, solid, 34);
  const background = floodFromBorder(W, H, (x, y) => {
    const i = y * W + x;
    if (solid[i]) return false;
    return !(lum(at(x, y)) < 40 && near[i] < 34);
  });
  // Only the robot: discards fragments of neighboring heads that fell inside the box.
  const fg = new Uint8Array(W * H);
  for (let i = 0; i < W * H; i++) fg[i] = background[i] ? 0 : 1;
  const robot = largestComponent(W, H, fg);
  const out = newPng(W, H);
  for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
    const i = y * W + x;
    if (!robot[i]) continue;
    const p = at(x, y);
    const o = i << 2;
    out.data[o] = p[0];
    out.data[o + 1] = p[1];
    out.data[o + 2] = p[2];
    out.data[o + 3] = 255;
  }
  // Canonical screen in the box's coordinates, shrunk to avoid the border.
  const s = head.screen;
  const screen = new Uint8Array(W * H);
  const sw = head.box.w;
  for (let y = 0; y < head.box.h; y++) for (let x = 0; x < sw; x++) {
    if (!s.mask[y * sw + x]) continue;
    const X = head.box.x0 - region.x0 + x;
    const Y = head.box.y0 - region.y0 + y;
    if (X >= 0 && Y >= 0 && X < W && Y < H) screen[Y * W + X] = 1;
  }
  const inset = erode(W, H, screen, SCREEN_INSET);
  const headBox = { x0: b.x0 - region.x0, y0: b.y0 - region.y0, w: b.w, h: b.h };
  return { img: out, W, H, region, screenBox: bbox(W, H, screen), inset, headBox };
}

/** Frame: canonical casing + the frame's screen content. */
function compose(canon, sheet, head, spec = {}) {
  const top = Math.round(canon.headBox.h * (spec.headroom ?? 0));
  const out = newPng(canon.W, canon.H + top);
  canon.img.data.copy(out.data, top * canon.W * 4);
  const cb = canon.screenBox;
  const fb = head.screen.box;
  if (spec.accents) {
    // Reference: the whole casing, because accents can extend beyond the screen.
    const hb = canon.headBox;
    const sb = head.box;
    for (let y = 0; y < canon.H + top; y++) for (let x = 0; x < canon.W; x++) {
      const u = Math.round(sb.x0 + ((x - hb.x0 + 0.5) / hb.w) * sb.w - 0.5);
      const v = Math.round(sb.y0 + ((y - top - hb.y0 + 0.5) / hb.h) * sb.h - 0.5);
      if (u < 0 || v < 0 || u >= sheet.width || v >= sheet.height) continue;
      const p = px(sheet, u, v);
      if (!isAccent(p)) continue;
      const o = (y * canon.W + x) << 2;
      out.data[o] = p[0];
      out.data[o + 1] = p[1];
      out.data[o + 2] = p[2];
      out.data[o + 3] = 255;
    }
  }
  for (let y = 0; y < canon.H; y++) for (let x = 0; x < canon.W; x++) {
    if (!canon.inset[y * canon.W + x]) continue;
    const u = Math.round(fb.x0 + ((x - cb.x0 + 0.5) / cb.w) * fb.w - 0.5);
    const v = Math.round(fb.y0 + ((y - cb.y0 + 0.5) / cb.h) * fb.h - 0.5);
    const p = px(sheet, Math.min(sheet.width - 1, Math.max(0, u)), Math.min(sheet.height - 1, Math.max(0, v)));
    const o = ((y + top) * canon.W + x) << 2;
    out.data[o] = p[0];
    out.data[o + 1] = p[1];
    out.data[o + 2] = p[2];
    out.data[o + 3] = 255;
  }
  return out;
}

/** The whole bubble: the opaque part of the sheet connected to its ivory (border and text included). */
function bubbleMask(img, ivory) {
  const { width: w, height: h } = img;
  const seen = new Uint8Array(w * h);
  const stack = [];
  for (let i = 0; i < w * h; i++) if (ivory[i]) {
    seen[i] = 1;
    stack.push(i);
  }
  while (stack.length) {
    const i = stack.pop();
    const x = i % w;
    const y = (i / w) | 0;
    for (const [nx, ny] of [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]) {
      if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
      const j = ny * w + nx;
      if (!seen[j] && img.data[(j << 2) + 3] > 160) {
        seen[j] = 1;
        stack.push(j);
      }
    }
  }
  return seen;
}

/**
 * Bubble layer in the canonical casing's coordinates, with `top` rows above.
 * Also returns the margins (in canonical pixels) needed beyond the frames'
 * crop: the bubble is taller and wider than the head.
 */
function bubbleLayer(canon, sheet, head, mask) {
  const hb = canon.headBox;
  const sb = head.box;
  const mb = bbox(sheet.width, sheet.height, mask);
  const toX = (u) => hb.x0 + ((u - sb.x0) / sb.w) * hb.w;
  const toY = (v) => hb.y0 + ((v - sb.y0) / sb.h) * hb.h;
  const top = Math.max(0, Math.ceil(canonBox.y0 - toY(mb.y0))) + 4;
  const side = Math.max(0, Math.ceil(canonBox.x0 - toX(mb.x0)), Math.ceil(toX(mb.x1 + 1) - (canonBox.x1 + 1))) + 2;
  // The canvas widens by `side` on each side: the bubble can extend beyond the canonical box.
  const out = newPng(canon.W + side * 2, canon.H + top);
  for (let y = 0; y < canon.H + top; y++) for (let x = 0; x < out.width; x++) {
    const u = Math.round(sb.x0 + ((x - side - hb.x0 + 0.5) / hb.w) * sb.w - 0.5);
    const v = Math.round(sb.y0 + ((y - top - hb.y0 + 0.5) / hb.h) * sb.h - 0.5);
    if (u < 0 || v < 0 || u >= sheet.width || v >= sheet.height || !mask[v * sheet.width + u]) continue;
    const p = px(sheet, u, v);
    const o = (y * out.width + x) << 2;
    out.data[o] = p[0];
    out.data[o + 1] = p[1];
    out.data[o + 2] = p[2];
    out.data[o + 3] = 255;
  }
  return { img: out, top, side };
}

/**
 * Crop to the canonical casing's box, the same for all frames:
 * same scale and same position. Any extra space at the top (headroom) stays
 * above the head, so the frame is taller but the head doesn't move.
 */
function trimAndScale(img, W, top = 0, side = 0, ox = 0) {
  const b = canonBox;
  const pad = Math.round(b.w * 0.04);
  const cw = b.w + pad * 2;
  const baseH = Math.round(((b.h + pad * 2) / cw) * W);
  const ch = b.h + pad * 2 + top;
  // `side`: extra margin on the left and right, at the same scale; `ox`: image
  // columns before the canonical origin.
  const crop = newPng(cw + side * 2, ch);
  for (let y = -top; y < b.h; y++) for (let x = -side; x < b.w + side; x++) {
    const sy = b.y0 + top + y;
    const sx = b.x0 + x + ox;
    if (sy < 0 || sx < 0 || sx >= img.width) continue;
    const s = (sy * img.width + sx) << 2;
    const d = ((y + top + pad) * crop.width + x + side + pad) << 2;
    img.data.copy(crop.data, d, s, s + 4);
  }
  return resizeArea(crop, W + Math.round(((side * 2) / cw) * W), baseH + Math.round((top / cw) * W));
}

/** Gaze direction: centroid of the pupils relative to the irises. */
function gaze(sheet, head) {
  const s = head.screen.box;
  let ix = 0, iy = 0, in_ = 0, px_ = 0, py_ = 0, pn = 0;
  for (let y = s.y0; y < s.y0 + s.h * 0.8; y++) for (let x = s.x0; x < s.x0 + s.w; x++) {
    const p = px(sheet, x, y);
    if (brightTurq(p) || (p[1] > 140 && p[2] > 130 && p[0] < 120)) {
      ix += x; iy += y; in_++;
    }
  }
  if (!in_) return null;
  ix /= in_; iy /= in_;
  // pupils: very dark pixels near the irises
  const turq = new Uint8Array(s.w * s.h);
  for (let y = 0; y < s.h; y++) for (let x = 0; x < s.w; x++) {
    const p = px(sheet, s.x0 + x, s.y0 + y);
    turq[y * s.w + x] = p[1] > 120 && p[2] > 110 && p[0] < 130 ? 1 : 0;
  }
  const near = distanceFrom(s.w, s.h, turq, 6);
  for (let y = 0; y < s.h * 0.8; y++) for (let x = 0; x < s.w; x++) {
    const p = px(sheet, s.x0 + x, s.y0 + y);
    if (lum(p) < 22 && near[y * s.w + x] <= 6) {
      // only if enclosed: at least one turquoise pixel to the left and right on the row
      px_ += s.x0 + x; py_ += s.y0 + y; pn++;
    }
  }
  if (!pn) return null;
  return { dx: (px_ / pn - ix) / s.w, dy: (py_ / pn - iy) / s.h };
}

const frames = {};
const overlays = {};
const sequences = {};
const gazes = {};
const hashes = new Map();
let canon = null;
let canonBox = null;

for (const [name, spec] of Object.entries(SHEETS)) {
  const sheet = load(path.join(src, `${spec.file}.png`));
  const bubbles = spec.bubble ? [] : null;
  const hs = heads(sheet, spec.accents, bubbles);
  // The glasses cut across the screen and distort its box: in that case it's
  // derived from the casing using the canonical casing's proportions.
  if (spec.accents && canon) for (const h of hs) {
    const c = canon.screenBox;
    const hb = canon.headBox;
    const want = { x0: h.box.x0 + ((c.x0 - hb.x0) / hb.w) * h.box.w, y0: h.box.y0 + ((c.y0 - hb.y0) / hb.h) * h.box.h, w: (c.w / hb.w) * h.box.w, h: (c.h / hb.h) * h.box.h };
    const s = h.screen.box;
    if (Math.abs(s.w - want.w) > want.w * 0.03 || Math.abs(s.h - want.h) > want.h * 0.03) {
      h.screen.box = Object.fromEntries(Object.entries(want).map(([k, v]) => [k, Math.round(v)]));
    }
  }
  if (!canon) {
    canon = canonical(sheet, hs[0]);
    const alpha = new Uint8Array(canon.W * canon.H);
    for (let i = 0; i < alpha.length; i++) alpha[i] = canon.img.data[(i << 2) + 3] > 0 ? 1 : 0;
    canonBox = bbox(canon.W, canon.H, alpha);
  }
  const ids = [];
  const top = Math.round(canon.headBox.h * (spec.headroom ?? 0));
  hs.forEach((head, i) => {
    const img = trimAndScale(compose(canon, sheet, head, spec), OUT_W, top);
    const hash = crypto.createHash("sha1").update(img.data).digest("hex");
    let id = hashes.get(hash);
    if (!id) {
      id = `${name}-${i + 1}`;
      hashes.set(hash, id);
      save(path.join(outDir, `${id}.png`), img);
      frames[id] = { file: `${id}.png`, width: img.width, height: img.height };
    }
    ids.push(id);
    if (!spec.durations) gazes[`${name}-${i + 1}`] = gaze(sheet, head);
  });
  sequences[name] = { frames: ids, durations: spec.durations };

  if (bubbles) for (const [id, i] of Object.entries(BUBBLES)) {
    const head = hs[i];
    // The bubble right above the head, not the one from the previous row.
    const b = bubbles
      .filter((c) => c.box.y1 < head.box.y0 && c.box.x0 < head.box.x1 && c.box.x1 > head.box.x0)
      .sort((p, q) => q.box.y1 - p.box.y1)[0];
    if (!b) throw new Error(`bubble not found above frame ${i + 1} of ${spec.file}`);
    const layer = bubbleLayer(canon, sheet, head, bubbleMask(sheet, b.mask));
    const img = trimAndScale(layer.img, OUT_W, layer.top, layer.side, layer.side);
    save(path.join(outDir, `${id}.png`), img);
    // `left`: offset relative to the frame, in pixels at width OUT_W.
    overlays[id] = { file: `${id}.png`, width: img.width, height: img.height, left: -(img.width - OUT_W) / 2 };
  }
}

fs.writeFileSync(path.join(outDir, "sprites.json"), JSON.stringify({ frames, overlays, sequences, gazes }, null, 2));
console.log("bubbles:", Object.entries(overlays).map(([k, o]) => `${k} ${o.width}x${o.height}`).join(", "));
console.log("frames:", Object.keys(frames).length);
for (const [k, g] of Object.entries(gazes)) console.log("gaze", k, g && `dx=${g.dx.toFixed(3)} dy=${g.dy.toFixed(3)}`);

// Previews on light and dark backgrounds at 48, 64 and 96 px.
if (previewDir) {
  fs.mkdirSync(previewDir, { recursive: true });
  for (const size of [48, 64, 96]) {
    const list = Object.values(frames);
    const cellW = size + 12;
    const cellH = Math.round(size * 0.85) + 12;
    for (const [bgName, bg] of [["light", [247, 247, 245]], ["dark", [22, 22, 22]]]) {
      const cols = 8;
      const rows = Math.ceil(list.length / cols);
      const sheet = newPng(cols * cellW, rows * cellH);
      for (let i = 0; i < sheet.data.length; i += 4) {
        sheet.data[i] = bg[0]; sheet.data[i + 1] = bg[1]; sheet.data[i + 2] = bg[2]; sheet.data[i + 3] = 255;
      }
      list.forEach((f, n) => {
        const img = resizeArea(load(path.join(outDir, f.file)), size, Math.round((f.height / f.width) * size));
        const ox = (n % cols) * cellW + 6;
        const oy = Math.floor(n / cols) * cellH + 6;
        for (let y = 0; y < img.height; y++) for (let x = 0; x < img.width; x++) {
          const s = (y * img.width + x) << 2;
          const d = ((oy + y) * sheet.width + ox + x) << 2;
          const a = img.data[s + 3] / 255;
          for (let k = 0; k < 3; k++) sheet.data[d + k] = Math.round(img.data[s + k] * a + sheet.data[d + k] * (1 - a));
        }
      });
      save(path.join(previewDir, `preview-${size}-${bgName}.png`), sheet);
    }
  }
}

// ---- Icons: app (1024) and menu bar (monochrome template) ------------------

{
  const sheet = load(path.join(src, `${SHEETS.rest.file}.png`));
  const master = compose(canon, sheet, heads(sheet)[0]);
  const alpha = new Uint8Array(master.width * master.height);
  for (let i = 0; i < alpha.length; i++) alpha[i] = master.data[(i << 2) + 3] > 0 ? 1 : 0;
  const b = bbox(master.width, master.height, alpha);

  // App icon: robot on a dark background with soft corners (macOS 824/1024 grid).
  const S = 1024;
  const icon = newPng(S, S);
  const inset = 100;
  const side = S - inset * 2;
  const radius = 185;
  for (let y = 0; y < S; y++) for (let x = 0; x < S; x++) {
    const lx = Math.max(inset + radius - x, 0, x - (S - inset - radius));
    const ly = Math.max(inset + radius - y, 0, y - (S - inset - radius));
    const inside = x >= inset && y >= inset && x < S - inset && y < S - inset && lx * lx + ly * ly <= radius * radius;
    if (!inside) continue;
    const t = (y - inset) / side;
    const o = (y * S + x) << 2;
    icon.data[o] = Math.round(46 - 20 * t);
    icon.data[o + 1] = Math.round(54 - 22 * t);
    icon.data[o + 2] = Math.round(70 - 28 * t);
    icon.data[o + 3] = 255;
  }
  const targetW = Math.round(side * 0.82);
  const crop = newPng(b.w, b.h);
  for (let y = 0; y < b.h; y++) master.data.copy(crop.data, y * b.w * 4, ((b.y0 + y) * master.width + b.x0) * 4, ((b.y0 + y) * master.width + b.x1 + 1) * 4);
  const robot = resizeArea(crop, targetW, Math.round((b.h / b.w) * targetW));
  const ox = Math.round((S - robot.width) / 2);
  const oy = Math.round((S - robot.height) / 2) + 10;
  for (let y = 0; y < robot.height; y++) for (let x = 0; x < robot.width; x++) {
    const s = (y * robot.width + x) << 2;
    const d = ((oy + y) * S + ox + x) << 2;
    const a = robot.data[s + 3] / 255;
    for (let k = 0; k < 3; k++) icon.data[d + k] = Math.round(robot.data[s + k] * a + icon.data[d + k] * (1 - a));
  }
  const iconDir = path.resolve("src-tauri/icons");
  fs.mkdirSync(iconDir, { recursive: true });
  save(path.join(iconDir, "source.png"), icon);

  // Menu bar: black silhouette, transparent screen, solid eyes.
  const tray = newPng(b.w, b.h);
  for (let y = 0; y < b.h; y++) for (let x = 0; x < b.w; x++) {
    const s = ((b.y0 + y) * master.width + b.x0 + x) << 2;
    if (!master.data[s + 3]) continue;
    const p = [master.data[s], master.data[s + 1], master.data[s + 2]];
    const screen = canon.inset[(b.y0 + y) * canon.W + b.x0 + x] || (lum(p) > 25 && lum(p) < 60 && !isIvory(p));
    const glyph = p[1] > 120 && p[2] > 110 && p[0] < 140;
    const d = (y * b.w + x) << 2;
    tray.data[d + 3] = screen && !glyph ? 0 : 255;
  }
  const trayH = 64;
  save(path.join(iconDir, "tray.png"), resizeArea(tray, Math.round((b.w / b.h) * trayH), trayH));
  console.log("icons generated in", iconDir);
}
