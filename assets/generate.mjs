// Generates the README visuals (light + dark).
// Usage: node assets/generate.mjs   (writes next to this file)
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const out = process.argv[2] || path.dirname(fileURLToPath(import.meta.url));
fs.mkdirSync(out, { recursive: true });

const SANS = "'Segoe UI', -apple-system, BlinkMacSystemFont, 'Helvetica Neue', Arial, sans-serif";
const MONO = "'Cascadia Mono', Consolas, 'SF Mono', Menlo, ui-monospace, monospace";

const T = {
  dark: {
    bg: '#18191d', surf: '#1e1f24', raised: '#26272d', sunk: '#101114',
    line: 'rgba(232,233,236,0.09)', line2: 'rgba(232,233,236,0.16)',
    text: '#e8e9ec', sec: '#a1a4ad', muted: '#6c707a',
    accent: '#409cff', accentInk: '#6fb4ff', accentFill: 'rgba(64,156,255,0.55)', accentSoft: 'rgba(64,156,255,0.14)',
    red: '#eb4040', keyTop: '#2c2d34', keyEdge: '#0c0d10',
  },
  light: {
    bg: '#f6f7f9', surf: '#ffffff', raised: '#eceef2', sunk: '#e4e6ea',
    line: 'rgba(22,23,26,0.10)', line2: 'rgba(22,23,26,0.18)',
    text: '#16171a', sec: '#555a64', muted: '#868b95',
    accent: '#2f86ea', accentInk: '#1a63bf', accentFill: 'rgba(47,134,234,0.62)', accentSoft: 'rgba(47,134,234,0.12)',
    red: '#d93636', keyTop: '#ffffff', keyEdge: '#c9ccd3',
  },
};

const esc = s => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
const svg = (w, h, body, label) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}" viewBox="0 0 ${w} ${h}" role="img" aria-label="${esc(label)}">\n${body}\n</svg>\n`;
const text = (x, y, s, { size = 16, fill, weight = 400, font = SANS, anchor = 'start', op } = {}) =>
  `<text x="${x}" y="${y}" font-family="${font}" font-size="${size}" font-weight="${weight}" fill="${fill}"${anchor !== 'start' ? ` text-anchor="${anchor}"` : ''}${op != null ? ` opacity="${op}"` : ''}>${esc(s)}</text>`;
const rect = (x, y, w, h, r, fill, extra = '') => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${r}" fill="${fill}"${extra ? ' ' + extra : ''}/>`;
const keycap = (c, x, y, label, w = 56, h = 48) =>
  `<g>${rect(x, y + 3, w, h, 10, c.keyEdge)}${rect(x, y, w, h, 10, c.keyTop, `stroke="${c.line2}" stroke-width="1"`)}` +
  text(x + w / 2, y + h / 2 + 7, label, { size: 19, fill: c.text, weight: 500, font: MONO, anchor: 'middle' }) + `</g>`;
const write = (name, s) => fs.writeFileSync(path.join(out, name), s);

// ───────────── banner ─────────────
function banner(c) {
  const W = 1280, H = 400, n = 16, sw = 64, gap = 8, x0 = 68, sy = 276, sh = 44;
  const keep = 5, firstKeep = n - keep;
  let b = rect(0, 0, W, H, 20, c.bg) + rect(0.5, 0.5, W - 1, H - 1, 20, 'none', `stroke="${c.line}"`);

  b += text(72, 128, 'LiteClip', { size: 60, fill: c.text, weight: 500 });
  b += text(74, 174, 'Replay-buffer screen recorder for Windows', { size: 23, fill: c.sec });
  b += text(74, 210, 'GPU encoding · saves without re-encoding · 4 MB · offline', { size: 15, fill: c.muted, font: MONO });

  // segment strip
  for (let i = 0; i < n; i++) {
    const x = x0 + i * (sw + gap);
    const saved = i >= firstKeep;
    const op = saved ? 1 : (0.28 + 0.72 * (i / (firstKeep - 1)) ** 1.4).toFixed(2);
    if (i === n - 1) {
      // segment being written
      b += rect(x, sy, sw, sh, 6, c.accentSoft, `stroke="${c.accent}" stroke-opacity="0.5"`);
      b += rect(x, sy, sw * 0.62, sh, 6, c.accentFill);
      const px = x + sw * 0.62;
      b += `<line x1="${px}" y1="${sy - 8}" x2="${px}" y2="${sy + sh + 8}" stroke="${c.red}" stroke-width="2"/>`;
      b += `<circle cx="${px}" cy="${sy - 10}" r="4" fill="${c.red}"/>`;
    } else if (saved) {
      b += rect(x, sy, sw, sh, 6, c.accentFill);
    } else {
      b += rect(x, sy, sw, sh, 6, c.raised, `stroke="${c.line}" opacity="${op}"`);
    }
  }
  // time axis
  const ty = sy + sh + 30;
  b += text(x0, ty, '−32 s', { size: 13, fill: c.muted, font: MONO });
  b += text(x0 + firstKeep * (sw + gap), ty, '−10 s', { size: 13, fill: c.accentInk, font: MONO });
  b += text(x0 + n * (sw + gap) - gap, ty, 'now', { size: 13, fill: c.red, font: MONO, anchor: 'end' });
  b += text(x0 + 4 * (sw + gap), ty, '2 s segments, oldest dropped', { size: 13, fill: c.muted, font: MONO });

  // bracket over the saved part
  const bx1 = x0 + firstKeep * (sw + gap), bx2 = x0 + n * (sw + gap) - gap, by = sy - 22;
  b += `<path d="M${bx1} ${by + 8} V${by} H${bx2} V${by + 8}" fill="none" stroke="${c.accent}" stroke-width="1.5"/>`;
  const mid = (bx1 + bx2) / 2;
  b += `<line x1="${mid}" y1="${by}" x2="${mid}" y2="172" stroke="${c.accent}" stroke-width="1.5" stroke-dasharray="3 4"/>`;

  // clip card
  const cx = bx1, cw = bx2 - bx1, cy = 96, ch = 76;
  b += rect(cx, cy, cw, ch, 12, c.surf, `stroke="${c.line2}"`);
  // file glyph
  b += `<g transform="translate(${cx + 20} ${cy + 22})" fill="none" stroke="${c.accentInk}" stroke-width="1.5" stroke-linejoin="round"><path d="M2 0h14l8 8v24H2z"/><path d="M16 0v8h8"/><path d="M10 14l7 4.5-7 4.5z" fill="${c.accentInk}" stroke="none"/></g>`;
  b += text(cx + 60, cy + 33, 'Clip 2026-11-27 20-14-03.mp4', { size: 15, fill: c.text, font: MONO });
  b += text(cx + 60, cy + 57, '10 s · stream copy · saved in < 1 s', { size: 13, fill: c.sec, font: MONO });

  // F8 key
  b += keycap(c, cx - 88, cy + 14, 'F8');
  b += `<path d="M${cx - 24} ${cy + 38} H${cx - 6}" stroke="${c.muted}" stroke-width="1.5"/><path d="M${cx - 11} ${cy + 33} l5 5 -5 5" fill="none" stroke="${c.muted}" stroke-width="1.5"/>`;
  return svg(W, H, b, 'LiteClip: keeps the last seconds of your screen in 2-second segments and saves them as an MP4 when you press F8');
}

// ───────────── how it works ─────────────
function flow(c) {
  const W = 1280, H = 470;
  let b = rect(0, 0, W, H, 20, c.bg) + rect(0.5, 0.5, W - 1, H - 1, 20, 'none', `stroke="${c.line}"`);
  const box = (x, y, w, h, title, sub, { strong = false } = {}) =>
    rect(x, y, w, h, 12, c.surf, `stroke="${strong ? c.accent : c.line2}"${strong ? ' stroke-opacity="0.6"' : ''}`) +
    text(x + 20, y + 32, title, { size: 16, fill: c.text, weight: 500 }) +
    (sub ? text(x + 20, y + 56, sub, { size: 13, fill: c.sec, font: MONO }) : '');
  const arrow = (d, label, lx, ly, color = c.muted) =>
    `<path d="${d}" fill="none" stroke="${color}" stroke-width="1.5"/>` +
    (label ? text(lx, ly, label, { size: 12, fill: c.muted, font: MONO }) : '');
  const head = (x, y, color = c.muted) => `<path d="M${x - 6} ${y - 5} L${x} ${y} L${x - 6} ${y + 5}" fill="none" stroke="${color}" stroke-width="1.5"/>`;

  b += text(48, 60, 'How a clip is made', { size: 20, fill: c.text, weight: 500 });
  b += text(48, 88, 'Video and audio are encoded once, while you play. Saving only copies what is already on disk.', { size: 14, fill: c.sec });

  // inputs
  b += box(48, 132, 236, 76, 'Screen', 'Desktop Duplication');
  b += box(48, 270, 236, 76, 'System sound', 'WASAPI loopback');
  b += box(48, 362, 236, 76, 'Microphone', 'any input device');

  // mixer
  b += box(316, 316, 216, 76, 'Audio mixer', 'Rust · cpal · 48 kHz');
  b += arrow('M284 308 H298 V346 H310', '', 0, 0) + head(316, 346);
  b += arrow('M284 400 H298 V362 H310', '', 0, 0) + head(316, 362);

  // ffmpeg container
  const fx = 624, fy = 120, fw = 300, fh = 318;
  b += rect(fx, fy, fw, fh, 16, c.sunk, `stroke="${c.line}"`);
  b += text(fx + 20, fy + 30, 'FFmpeg', { size: 13, fill: c.muted, font: MONO });
  const step = (i, t, s) => {
    const y = fy + 48 + i * 86;
    return rect(fx + 16, y, fw - 32, 70, 10, c.surf, `stroke="${c.line2}"`) +
      text(fx + 36, y + 30, t, { size: 15, fill: c.text, weight: 500 }) +
      text(fx + 36, y + 52, s, { size: 12, fill: c.sec, font: MONO });
  };
  b += step(0, 'Capture', 'ddagrab, or GDI fallback');
  b += step(1, 'Encode on the GPU', 'NVENC · Quick Sync · AMF');
  b += step(2, 'Split into segments', '2 s MPEG-TS files');
  b += arrow(`M284 170 H${fx + 10}`, '', 0, 0) + head(fx + 16, 170);
  b += arrow(`M532 354 H596 V${fy + 48 + 86 + 35} H${fx + 10}`, 'PCM pipe', 538, 344) + head(fx + 16, fy + 48 + 86 + 35);
  // inner arrows
  for (const i of [0, 1]) {
    const y = fy + 48 + i * 86 + 70;
    b += `<path d="M${fx + fw / 2} ${y} V${y + 16}" stroke="${c.muted}" stroke-width="1.5"/>`;
  }

  // buffer
  const bx = 976, bw = 256;
  b += rect(bx, 132, bw, 132, 12, c.surf, `stroke="${c.line2}"`);
  b += text(bx + 20, 164, 'Replay buffer', { size: 16, fill: c.text, weight: 500 });
  b += text(bx + 20, 188, 'last N seconds on disk', { size: 13, fill: c.sec, font: MONO });
  for (let i = 0; i < 8; i++) {
    const x = bx + 20 + i * 27;
    b += rect(x, 210, 23, 30, 4, i >= 5 ? c.accentFill : c.raised, i >= 5 ? '' : `stroke="${c.line}" opacity="${(0.35 + i * 0.13).toFixed(2)}"`);
  }
  b += arrow(`M${fx + fw} ${fy + 48 + 172 + 35} H950 V198 H${bx - 6}`, '', 0, 0) + head(bx, 198);

  // save
  b += keycap(c, bx, 300, 'F8');
  b += text(bx + 72, 324, 'join + trim', { size: 13, fill: c.sec, font: MONO });
  b += text(bx + 72, 344, '-c copy', { size: 13, fill: c.accentInk, font: MONO });
  b += `<path d="M${bx + 104} 264 V292" stroke="${c.accent}" stroke-width="1.5" stroke-dasharray="3 4"/>`;
  b += rect(bx, 368, bw, 70, 12, c.surf, `stroke="${c.accent}" stroke-opacity="0.6"`);
  b += text(bx + 20, 398, 'Clip … .mp4', { size: 15, fill: c.text, weight: 500, font: MONO });
  b += text(bx + 20, 420, 'Videos\\LiteClip · < 1 s', { size: 13, fill: c.sec, font: MONO });
  return svg(W, H, b, 'How LiteClip works: FFmpeg captures and encodes the screen into 2-second segments while Rust mixes system sound and microphone; F8 joins the latest segments with stream copy into an MP4');
}

// ───────────── numbers ─────────────
function stats(c) {
  const W = 1280, H = 300;
  let b = rect(0, 0, W, H, 20, c.bg) + rect(0.5, 0.5, W - 1, H - 1, 20, 'none', `stroke="${c.line}"`);
  // left: stats
  const stat = (x, fig, unit, label) =>
    `<text x="${x}" y="122" font-family="${SANS}" fill="${c.text}"><tspan font-size="52" font-weight="500">${esc(fig)}</tspan><tspan font-size="22" fill="${c.sec}" dx="6">${esc(unit)}</tspan></text>` +
    text(x, 154, label, { size: 14, fill: c.sec });
  b += stat(56, '4', 'MB', 'single executable');
  b += stat(232, '<1', 's', 'to save a clip');
  b += stat(408, '0', 'requests', 'no network at all');
  b += text(56, 236, 'Measured on an Intel i5-1035G4 laptop', { size: 13, fill: c.muted, font: MONO });
  b += text(56, 258, 'with Iris Plus graphics, 1080p at 30 fps', { size: 13, fill: c.muted, font: MONO });

  // divider
  b += `<line x1="640" y1="48" x2="640" y2="252" stroke="${c.line}"/>`;

  // right: bar chart, % of one CPU core
  const cx = 688, cw = 536, top = 72;
  b += text(cx, 64, 'CPU while recording', { size: 16, fill: c.text, weight: 500 });
  b += text(cx + cw, 64, '% of one core', { size: 13, fill: c.muted, font: MONO, anchor: 'end' });
  const ax = cx, aw = cw, gy1 = top + 24, gy2 = 236;
  for (const p of [0, 25, 50, 75, 100]) {
    const x = ax + (aw * p) / 100;
    b += `<line x1="${x}" y1="${gy1}" x2="${x}" y2="${gy2}" stroke="${c.line}"/>`;
    b += text(x, gy2 + 22, p + '%', { size: 12, fill: c.muted, font: MONO, anchor: p === 0 ? 'start' : p === 100 ? 'end' : 'middle' });
  }
  const bar = (y, label, v, color) =>
    text(ax, y - 10, label, { size: 14, fill: c.sec }) +
    rect(ax, y, (aw * v) / 100, 28, 4, color) +
    text(ax + (aw * v) / 100 + 10, y + 20, `≈${v}%`, { size: 14, fill: c.text, font: MONO });
  b += bar(130, 'ffmpeg (capture + encode)', 17, c.accent);
  b += bar(196, 'LiteClip window', 2, c.muted);
  return svg(W, H, b, 'LiteClip numbers: 4 MB executable, under 1 second to save, no network; while recording 1080p30 on an i5-1035G4, ffmpeg uses about 17% of one core and the UI about 2%');
}

// ───────────── editor illustration (app is dark-only) ─────────────
function editor() {
  const c = T.dark;
  const W = 1280, H = 820;
  const win = { x: 40, y: 40, w: 1200, h: 740 };
  let b = rect(0, 0, W, H, 20, '#0e0f12');
  b += rect(win.x, win.y + 10, win.w, win.h, 12, '#000', 'opacity="0.35"');
  b += rect(win.x, win.y, win.w, win.h, 12, c.bg, `stroke="${c.line2}"`);
  // title bar
  b += `<path d="M${win.x + 12} ${win.y} H${win.x + win.w - 12} Q${win.x + win.w} ${win.y} ${win.x + win.w} ${win.y + 12} V${win.y + 36} H${win.x} V${win.y + 12} Q${win.x} ${win.y} ${win.x + 12} ${win.y}Z" fill="#141519"/>`;
  b += text(win.x + 16, win.y + 23, 'LiteClip', { size: 12, fill: c.sec });
  const wx = win.x + win.w;
  b += `<g stroke="${c.sec}" stroke-width="1" fill="none"><path d="M${wx - 122} ${win.y + 18} h10"/><rect x="${wx - 76}" y="${win.y + 13}" width="10" height="10"/><path d="M${wx - 30} ${win.y + 13} l10 10 M${wx - 20} ${win.y + 13} l-10 10"/></g>`;
  // top bar
  const tb = win.y + 36;
  b += rect(win.x, tb, win.w, 48, 0, '#121317');
  const L = win.x + 20, ty = tb + 30;
  b += text(L, ty + 1, 'LiteClip', { size: 18, fill: c.accent, weight: 600 });
  b += `<line x1="${L + 86}" y1="${tb + 14}" x2="${L + 86}" y2="${tb + 34}" stroke="${c.line2}"/>`;
  b += `<circle cx="${L + 106}" cy="${ty - 5}" r="5" fill="${c.red}"/>`;
  b += text(L + 118, ty, 'Buffering', { size: 14, fill: c.red, weight: 500 });
  b += text(L + 200, ty, '00:12:41', { size: 14, fill: c.sec, font: MONO });
  const btn = (x, y, w, label, fill = '#2a2b31', color = c.text, h = 26) =>
    rect(x, y, w, h, 6, fill, `stroke="${c.line}"`) + text(x + w / 2, y + h / 2 + 5, label, { size: 13, fill: color, anchor: 'middle' });
  b += btn(L + 280, tb + 11, 56, 'Stop');
  b += btn(L + 346, tb + 11, 88, 'Save clip', 'rgba(64,156,255,0.45)', '#fff');
  // tabs
  const tabs = ['Clips', 'Editor', 'Settings'];
  let tx = wx - 20 - 3 * 76;
  tabs.forEach(t => {
    if (t === 'Editor') b += rect(tx, tb + 11, 68, 26, 6, 'rgba(64,156,255,0.22)');
    b += text(tx + 34, tb + 29, t, { size: 13, fill: t === 'Editor' ? c.text : c.sec, anchor: 'middle' });
    tx += 76;
  });
  // header
  const hy = tb + 48 + 40;
  b += text(L, hy, 'Clip 2026-11-27 20-14-03.mp4', { size: 18, fill: c.text, weight: 500 });
  b += text(L + 290, hy, '0:30.0 · 1920×1080', { size: 13, fill: c.muted });
  b += btn(wx - 20 - 60, hy - 18, 60, 'Close');
  // preview (stylised game frame)
  const pw = 640, ph = 360, pxx = win.x + (win.w - pw) / 2, py = hy + 20;
  b += `<defs><clipPath id="pv"><rect x="${pxx}" y="${py}" width="${pw}" height="${ph}" rx="4"/></clipPath>
<linearGradient id="sky" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#2a3a4f"/><stop offset="1" stop-color="#4b5f74"/></linearGradient></defs>`;
  b += `<g clip-path="url(#pv)">${rect(pxx, py, pw, ph, 0, 'url(#sky)')}`;
  b += `<path d="M${pxx} ${py + 230} L${pxx + 120} ${py + 170} L${pxx + 230} ${py + 215} L${pxx + 360} ${py + 150} L${pxx + 470} ${py + 205} L${pxx + 640} ${py + 160} V${py + ph} H${pxx}Z" fill="#33414f"/>`;
  b += `<path d="M${pxx} ${py + 290} L${pxx + 180} ${py + 250} L${pxx + 330} ${py + 275} L${pxx + 520} ${py + 240} L${pxx + 640} ${py + 262} V${py + ph} H${pxx}Z" fill="#232c35"/>`;
  b += `<path d="M${pxx + 400} ${py + ph} L${pxx + 470} ${py + 268} L${pxx + 560} ${py + 262} L${pxx + 600} ${py + ph}Z" fill="#151a20"/>`;
  const ccx = pxx + pw / 2, ccy = py + ph / 2;
  b += `<g stroke="#e8e9ec" stroke-width="2" opacity="0.85"><path d="M${ccx - 14} ${ccy} h8 M${ccx + 6} ${ccy} h8 M${ccx} ${ccy - 14} v8 M${ccx} ${ccy + 6} v8"/></g>`;
  b += text(pxx + pw - 20, py + ph - 20, '24 / 90', { size: 20, fill: '#e8e9ec', font: MONO, anchor: 'end', op: 0.85 });
  b += `<rect x="${pxx + 20}" y="${py + ph - 34}" width="120" height="8" rx="2" fill="#e8e9ec" opacity="0.25"/><rect x="${pxx + 20}" y="${py + ph - 34}" width="84" height="8" rx="2" fill="#e8e9ec" opacity="0.8"/>`;
  b += `</g>`;
  // controls
  const cy = py + ph + 20;
  let x = L;
  const ctl = [['Pause', 56], ['-1s', 40], ['+1s', 40]];
  ctl.forEach(([l, w]) => { b += btn(x, cy, w, l); x += w + 8; });
  b += text(x + 6, cy + 18, '0:14.2 / 0:30.0', { size: 13, fill: c.text, font: MONO }); x += 132;
  b += rect(x, cy + 6, 14, 14, 3, 'rgba(64,156,255,0.45)') + `<path d="M${x + 3.5} ${cy + 13} l2.5 2.5 4.5 -5" fill="none" stroke="#fff" stroke-width="1.5"/>`;
  b += text(x + 22, cy + 18, 'Preview sound', { size: 13, fill: c.text }); x += 132;
  [['Split here', 80], ['Cut this part', 104], ['Trim start', 80], ['Trim end', 74], ['Reset', 56]].forEach(([l, w]) => { b += btn(x, cy, w, l); x += w + 8; });
  // timeline
  const tly = cy + 42, tlw = win.w - 40, dur = 30;
  b += rect(L, tly, tlw, 46, 4, '#101114');
  const xs = t => L + (t / dur) * tlw;
  const segs = [[0, 6.4, false], [6.4, 11.8, true], [11.8, 30, false]];
  for (const [s, e, cut] of segs) {
    b += rect(xs(s) + 1, tly + 6, xs(e) - xs(s) - 2, 34, 3, cut ? '#5a2828' : c.accentFill);
    if (cut) b += text((xs(s) + xs(e)) / 2, tly + 27, 'cut', { size: 11, fill: '#dc8c8c', anchor: 'middle' });
  }
  for (const s of [6.4, 11.8]) b += `<line x1="${xs(s)}" y1="${tly}" x2="${xs(s)}" y2="${tly + 46}" stroke="#ffd250" stroke-width="2"/>`;
  b += `<line x1="${xs(14.2)}" y1="${tly}" x2="${xs(14.2)}" y2="${tly + 46}" stroke="#fff" stroke-width="2"/><circle cx="${xs(14.2)}" cy="${tly + 3}" r="4" fill="#fff"/>`;
  // export row
  const ey = tly + 46 + 34;
  b += `<line x1="${L}" y1="${ey - 16}" x2="${L + tlw}" y2="${ey - 16}" stroke="${c.line}"/>`;
  b += text(L, ey + 4, 'Export', { size: 14, fill: c.text, weight: 600 });
  b += text(L + 58, ey + 4, '2 part(s) · output length 0:24.6', { size: 13, fill: c.muted });
  const slider = (x, y, w, f, v) => rect(x, y + 9, w, 6, 3, '#2a2b31') + rect(x, y + 9, w * f, 6, 3, 'rgba(64,156,255,0.6)') + `<circle cx="${x + w * f}" cy="${y + 12}" r="7" fill="#c9ccd3"/>` + text(x + w + 12, y + 17, v, { size: 13, fill: c.text, font: MONO });
  const r1 = ey + 22;
  b += text(L, r1 + 17, 'Speed', { size: 13, fill: c.sec }) + slider(L + 90, r1, 180, 0.5, '1.00×');
  b += text(L + 360, r1 + 17, 'Volume', { size: 13, fill: c.sec }) + slider(L + 450, r1, 180, 1 / 3, '100%');
  const combo = (x, y, w, v) => rect(x, y, w, 26, 6, '#2a2b31', `stroke="${c.line}"`) + text(x + 10, y + 18, v, { size: 13, fill: c.text }) + `<path d="M${x + w - 18} ${y + 11} l4 4 4 -4" fill="none" stroke="${c.sec}" stroke-width="1.5"/>`;
  const r2 = r1 + 36;
  b += text(L, r2 + 17, 'Resolution', { size: 13, fill: c.sec }) + combo(L + 90, r2, 180, 'Original');
  b += text(L + 360, r2 + 17, 'Frame rate', { size: 13, fill: c.sec }) + combo(L + 450, r2, 180, '60 fps');
  b += btn(wx - 20 - 96, r2 - 4, 96, 'Export', 'rgba(64,156,255,0.45)', '#fff', 30);
  return svg(W, H, b, 'Illustration of the LiteClip editor: preview, a timeline with one section cut, and export settings');
}

for (const [name, c] of Object.entries(T)) {
  write(`banner-${name}.svg`, banner(c));
  write(`how-it-works-${name}.svg`, flow(c));
  write(`numbers-${name}.svg`, stats(c));
}
write('editor.svg', editor());
console.log('ok');
