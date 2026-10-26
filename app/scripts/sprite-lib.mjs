// Utilities for cleaning up the companion's ImageGen sheets (pixel art).
import fs from "node:fs";
import { PNG } from "pngjs";

export function load(path) {
  return PNG.sync.read(fs.readFileSync(path));
}

export function save(path, png) {
  fs.writeFileSync(path, PNG.sync.write(png));
}

export function px(img, x, y) {
  const i = (img.width * y + x) << 2;
  return [img.data[i], img.data[i + 1], img.data[i + 2], img.data[i + 3]];
}

export function crop(img, x0, y0, w, h) {
  const out = new PNG({ width: w, height: h });
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const s = ((y0 + y) * img.width + (x0 + x)) << 2;
      const d = (y * w + x) << 2;
      for (let k = 0; k < 4; k++) out.data[d + k] = img.data[s + k];
    }
  return out;
}

export const isIvory = ([r, g, b]) => r > 150 && g > 130 && b > 100 && r >= b && r - b < 90;
export const isTurq = ([r, g, b]) => g > 120 && b > 110 && r < 140 && g - r > 50;
export const lum = ([r, g, b]) => 0.299 * r + 0.587 * g + 0.114 * b;

/** Estimates the pixel-art grid's step and phase along an axis from color edges. */
export function estimateGrid(img, axis, from, to) {
  const n = axis === "x" ? img.width : img.height;
  const edge = new Float64Array(n);
  const m = axis === "x" ? img.height : img.width;
  for (let i = 1; i < n; i++) {
    let s = 0;
    for (let j = from; j < Math.min(to, m); j += 2) {
      const a = axis === "x" ? px(img, i, j) : px(img, j, i);
      const b = axis === "x" ? px(img, i - 1, j) : px(img, j, i - 1);
      s += Math.abs(a[0] - b[0]) + Math.abs(a[1] - b[1]) + Math.abs(a[2] - b[2]);
    }
    edge[i] = s;
  }
  let best = { score: -1, step: 0, phase: 0 };
  for (let step = 8; step <= 24; step += 0.02) {
    for (let phase = 0; phase < step; phase += 0.25) {
      let score = 0;
      let count = 0;
      for (let p = phase; p < n; p += step) {
        const i = Math.round(p);
        if (i > 0 && i < n) {
          score += edge[i];
          count++;
        }
      }
      // normalize against the mean so small steps aren't favored
      score = score / Math.max(1, count);
      if (score > best.score) best = { score, step, phase };
    }
  }
  return best;
}

export function newPng(w, h) {
  const p = new PNG({ width: w, height: h });
  p.data.fill(0);
  return p;
}

/** 4-connected flood fill from the borders over `passable(x, y)`. */
export function floodFromBorder(w, h, passable) {
  const seen = new Uint8Array(w * h);
  const stack = [];
  const push = (x, y) => {
    const i = y * w + x;
    if (!seen[i] && passable(x, y)) {
      seen[i] = 1;
      stack.push(i);
    }
  };
  for (let x = 0; x < w; x++) {
    push(x, 0);
    push(x, h - 1);
  }
  for (let y = 0; y < h; y++) {
    push(0, y);
    push(w - 1, y);
  }
  while (stack.length) {
    const i = stack.pop();
    const x = i % w;
    const y = (i / w) | 0;
    if (x > 0) push(x - 1, y);
    if (x < w - 1) push(x + 1, y);
    if (y > 0) push(x, y - 1);
    if (y < h - 1) push(x, y + 1);
  }
  return seen;
}

/** Largest connected component of `mask`. */
export function largestComponent(w, h, mask) {
  const label = new Int32Array(w * h).fill(-1);
  let best = { id: -1, size: 0 };
  let id = 0;
  for (let s = 0; s < w * h; s++) {
    if (!mask[s] || label[s] >= 0) continue;
    const stack = [s];
    label[s] = id;
    let size = 0;
    while (stack.length) {
      const i = stack.pop();
      size++;
      const x = i % w;
      const y = (i / w) | 0;
      for (const [nx, ny] of [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]) {
        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
        const j = ny * w + nx;
        if (mask[j] && label[j] < 0) {
          label[j] = id;
          stack.push(j);
        }
      }
    }
    if (size > best.size) best = { id, size };
    id++;
  }
  const out = new Uint8Array(w * h);
  for (let i = 0; i < w * h; i++) out[i] = label[i] === best.id ? 1 : 0;
  return out;
}

export function bbox(w, h, mask) {
  let x0 = w, y0 = h, x1 = -1, y1 = -1;
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++)
      if (mask[y * w + x]) {
        if (x < x0) x0 = x;
        if (x > x1) x1 = x;
        if (y < y0) y0 = y;
        if (y > y1) y1 = y;
      }
  return { x0, y0, x1, y1, w: x1 - x0 + 1, h: y1 - y0 + 1 };
}

/** Distance (BFS, in pixels) from the pixels of `mask`, capped at `max`. */
export function distanceFrom(w, h, mask, max) {
  const d = new Uint16Array(w * h).fill(65535);
  let frontier = [];
  for (let i = 0; i < w * h; i++) if (mask[i]) {
    d[i] = 0;
    frontier.push(i);
  }
  for (let k = 1; k <= max && frontier.length; k++) {
    const next = [];
    for (const i of frontier) {
      const x = i % w;
      const y = (i / w) | 0;
      for (const [nx, ny] of [[x - 1, y], [x + 1, y], [x, y - 1], [x, y + 1]]) {
        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
        const j = ny * w + nx;
        if (d[j] === 65535) {
          d[j] = k;
          next.push(j);
        }
      }
    }
    frontier = next;
  }
  return d;
}

export function erode(w, h, mask, r) {
  const inv = new Uint8Array(w * h);
  for (let i = 0; i < w * h; i++) inv[i] = mask[i] ? 0 : 1;
  const d = distanceFrom(w, h, inv, r + 1);
  const out = new Uint8Array(w * h);
  for (let i = 0; i < w * h; i++) out[i] = mask[i] && d[i] > r ? 1 : 0;
  return out;
}

/** Area resize (box filter) with premultiplied alpha. */
export function resizeArea(src, W, H) {
  const out = newPng(W, H);
  const sx = src.width / W;
  const sy = src.height / H;
  for (let Y = 0; Y < H; Y++)
    for (let X = 0; X < W; X++) {
      const xa = X * sx, xb = (X + 1) * sx, ya = Y * sy, yb = (Y + 1) * sy;
      let r = 0, g = 0, b = 0, a = 0, area = 0;
      for (let y = Math.floor(ya); y < Math.ceil(yb); y++)
        for (let x = Math.floor(xa); x < Math.ceil(xb); x++) {
          const wx = Math.min(xb, x + 1) - Math.max(xa, x);
          const wy = Math.min(yb, y + 1) - Math.max(ya, y);
          const wgt = wx * wy;
          const i = (y * src.width + x) << 2;
          const al = src.data[i + 3] / 255;
          r += src.data[i] * al * wgt;
          g += src.data[i + 1] * al * wgt;
          b += src.data[i + 2] * al * wgt;
          a += al * wgt;
          area += wgt;
        }
      const o = (Y * W + X) << 2;
      if (a > 0) {
        out.data[o] = Math.round(r / a);
        out.data[o + 1] = Math.round(g / a);
        out.data[o + 2] = Math.round(b / a);
      }
      out.data[o + 3] = Math.round((a / area) * 255);
    }
  return out;
}
