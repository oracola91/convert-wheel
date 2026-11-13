// The Mark Up window: the picture, and tools to draw on it (pen, highlighter, arrow,
// line, box, circle, words, numbered steps) or black parts of it out. Select picks a
// mark already drawn to move it (drag or arrow keys), recolour or resize it, delete it,
// or retype its words (double-click); Undo steps back through every change. Sizes work
// like Word's, in points as the picture looks in this window; colours are six quick
// ones, or any from the palette or typed as a code. Words are typed straight onto the
// picture: click where they go, type, Enter. What is drawn is kept as shapes in the
// picture's own pixels; the app checks them and ImageMagick draws them the same way
// (src-tauri/src/convert/markup.rs), the blacked-out parts last, over everything.
// For Black Out on a video, the same window shows a frame of it with only Select and
// Black out: each box covers that place for the whole video (ffmpeg fills it).
if (!window.__TAURI__) throw new Error('Pirouette: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const stage = $('stage');
const canvas = $('photo');
const ctx = canvas.getContext('2d');
const measure = document.createElement('canvas').getContext('2d');
const apply = $('apply');
const undo = $('undo');
const message = $('message');
const colours = $('colours');
const sizeValue = $('sizeValue');
const sizeOpen = $('sizeOpen');
const sizeList = $('sizes');
const shapeTool = $('shapeTool');
const shapeOpen = $('shapeOpen');
const shapeList = $('shapeList');
const palette = $('palette');
const grid = $('grid');
const code = $('code');
const tools = shapeTool.closest('.group');
const POPS = [sizeList, shapeList, palette];

// The quick colours, the same as COLOURS in markup.rs, by the names sent back. Any
// other colour is sent as its #rrggbb code.
const COLOURS = { red: '#e5322d', yellow: '#f5c400', blue: '#1d6fe5', green: '#21a148', black: '#000000', white: '#ffffff' };
const hexOf = (c) => COLOURS[c] ?? c;
const FONT = 'Arial, Helvetica, "DejaVu Sans", sans-serif';
const MAX_POINTS = 4000;
const MAX_WORDS = 200;
const MAX_STEP = 99;
const MAX_HISTORY = 200;
// The highlighter's see-through alpha, as HIGHLIGHT_ALPHA in markup.rs (40%).
const HIGHLIGHT_ALPHA = '66';
// Palette colours picked lately, kept next to the quick ones.
const RECENT = 3;

// The tools in the order the arrow keys go through them. The shapes share a button.
const TOOLS = ['select', 'pen', 'highlight', 'arrow', 'line', 'box', 'circle', 'text', 'number', 'redact'];
// The tools on offer: all of them, or for a video only these.
let toolOrder = TOOLS;
const VIDEO_TOOLS = ['select', 'redact'];
const SHAPES = { arrow: 'Arrow', line: 'Line', box: 'Box', circle: 'Circle' };

// Which size each kind of mark takes. The lists are what to pick, as in Word (font
// sizes for words and steps, line weights for the rest); any other whole number can
// be typed, within LIMITS. RANGE is what the app takes, in the picture's pixels.
const SIZE_OF = { pen: 'line', arrow: 'line', line: 'line', box: 'line', circle: 'line', highlight: 'marker', text: 'text', number: 'number' };
const SIZES = {
  text: [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72],
  line: [1, 2, 3, 4, 6, 8, 10, 12, 16, 20],
  marker: [6, 8, 10, 12, 16, 20, 24, 32, 40],
  number: [10, 12, 14, 16, 20, 24, 28, 36, 48],
};
const LIMITS = { text: [1, 400], line: [1, 100], marker: [1, 200], number: [4, 400] };
const RANGE = { text: [8, 1000], line: [1, 200], marker: [1, 600], number: [8, 1000] };
const NAMES = { text: 'words', line: 'line', marker: 'highlighter', number: 'steps' };
const sizes = { text: 24, line: 3, marker: 16, number: 20 };

// The palette: a row of greys, then ten hues in five shades, light to dark.
function hsl(h, s, l) {
  const a = (s / 100) * Math.min(l / 100, 1 - l / 100);
  const part = (n) => {
    const k = (n + h / 30) % 12;
    const c = l / 100 - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, '0');
  };
  return `#${part(0)}${part(8)}${part(4)}`;
}
const PALETTE = [
  ...Array.from({ length: 10 }, (_, i) => `#${Math.round(255 * (1 - i / 9)).toString(16).padStart(2, '0').repeat(3)}`),
  ...[88, 74, 58, 44, 30].flatMap((l) => [0, 24, 45, 58, 100, 150, 185, 212, 265, 315].map((h) => hsl(h, 80, l))),
];

// { name, width, height, preview } from the app, once loaded.
let session = null;
const image = new Image();
let shapes = [];
// The shapes as they were before each change, for Undo.
let history = [];
let drawing = null;
// The mark picked with Select (its index, or -1), and a drag moving it.
let selected = -1;
let moving = null;
// The words being typed, if any: { box, at, size, index } with `at` and `size` in the
// picture's pixels, and `index` the words being retyped (or -1 for new ones).
let typing = null;
let tool = 'pen';
let colour = 'red';
let recent = [];
// The button that opened a list or the palette, to go back to.
let opener = null;
// Where the picture sits on the canvas: picture pixels times `scale`, from left, top.
let view = { scale: 1, left: 0, top: 0, dpr: 1 };

function say(text) {
  message.textContent = text;
  message.hidden = !text;
}

// The size in use now: the words being typed, the picked mark's, or the tool's.
function sizeKind() {
  if (typing) return 'text';
  return SIZE_OF[tool === 'select' ? shapes[selected]?.kind : tool] ?? null;
}

// A size in points as the picture looks in this window (a point is 4/3 of a screen
// pixel), in the picture's own pixels within what the app takes, and back. What is
// seen is what is saved, whatever the picture's size.
function pixels(points, kind) {
  const [min, max] = RANGE[kind];
  return Math.min(max, Math.max(min, Math.round((points * 4) / 3 / view.scale)));
}
function toPoints(px, kind) {
  const [low, high] = LIMITS[kind];
  return Math.min(high, Math.max(low, Math.round((px * view.scale * 3) / 4)));
}
const sizeNow = (kind) => pixels(sizes[kind], kind);
const sizeField = (s) => (s.kind === 'text' || s.kind === 'number' ? 'size' : 'width');

function remember() {
  history.push(JSON.stringify(shapes));
  if (history.length > MAX_HISTORY) history.shift();
}

function layout() {
  if (!session || !image.complete) return;
  const box = stage.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.round(box.width * dpr);
  canvas.height = Math.round(box.height * dpr);
  canvas.style.left = '0';
  canvas.style.top = '0';
  canvas.style.width = `${box.width}px`;
  canvas.style.height = `${box.height}px`;
  const scale = Math.min(box.width / session.width, box.height / session.height);
  view = { scale, left: (box.width - session.width * scale) / 2, top: (box.height - session.height * scale) / 2, dpr };
  redraw();
  styleTyping();
}

// The pointer in the picture's pixels, kept inside it.
function toPicture(event) {
  const r = canvas.getBoundingClientRect();
  const clamp = (v, max) => Math.min(Math.max(Math.round(v), 0), max);
  return [
    clamp((event.clientX - r.left - view.left) / view.scale, session.width),
    clamp((event.clientY - r.top - view.top) / view.scale, session.height),
  ];
}

const corners = (from, to) => [Math.min(from[0], to[0]), Math.min(from[1], to[1]), Math.max(from[0], to[0]), Math.max(from[1], to[1])];

// Whether a colour is light, so dark digits read on it (as light() in markup.rs).
function light(hex) {
  const part = (at) => parseInt(hex.slice(at, at + 2), 16) || 0;
  return 0.299 * part(1) + 0.587 * part(3) + 0.114 * part(5) > 160;
}

// The same arrow head as arrow_head in markup.rs.
function arrowHead(from, to, width) {
  const angle = Math.atan2(to[1] - from[1], to[0] - from[0]);
  const length = Math.max(width * 4, 12);
  const spread = (25 * Math.PI) / 180;
  const corner = (a) => [to[0] - length * Math.cos(a), to[1] - length * Math.sin(a)];
  const back = length * Math.cos(spread) * 0.8;
  return { end: [to[0] - back * Math.cos(angle), to[1] - back * Math.sin(angle)], head: [to, corner(angle - spread), corner(angle + spread)] };
}

function paint(s) {
  ctx.lineCap = 'round';
  ctx.lineJoin = 'round';
  const ink = hexOf(s.colour ?? 'black');
  const [x0, y0, x1, y1] = s.from ? corners(s.from, s.to) : [];
  if (s.kind === 'pen' || s.kind === 'highlight') {
    const c = s.kind === 'highlight' ? ink + HIGHLIGHT_ALPHA : ink;
    ctx.strokeStyle = c;
    ctx.fillStyle = c;
    ctx.lineWidth = s.width;
    if (s.points.length === 1) {
      ctx.beginPath();
      ctx.arc(s.points[0][0], s.points[0][1], s.width / 2, 0, Math.PI * 2);
      ctx.fill();
    } else {
      ctx.beginPath();
      ctx.moveTo(...s.points[0]);
      for (const p of s.points.slice(1)) ctx.lineTo(...p);
      ctx.stroke();
    }
  } else if (s.kind === 'arrow') {
    const { end, head } = arrowHead(s.from, s.to, s.width);
    ctx.strokeStyle = ink;
    ctx.fillStyle = ink;
    ctx.lineWidth = s.width;
    ctx.beginPath();
    ctx.moveTo(...s.from);
    ctx.lineTo(...end);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(...head[0]);
    ctx.lineTo(...head[1]);
    ctx.lineTo(...head[2]);
    ctx.closePath();
    ctx.fill();
  } else if (s.kind === 'line') {
    ctx.strokeStyle = ink;
    ctx.lineWidth = s.width;
    ctx.beginPath();
    ctx.moveTo(...s.from);
    ctx.lineTo(...s.to);
    ctx.stroke();
  } else if (s.kind === 'box') {
    ctx.strokeStyle = ink;
    ctx.lineWidth = s.width;
    ctx.strokeRect(x0, y0, x1 - x0, y1 - y0);
  } else if (s.kind === 'circle') {
    ctx.strokeStyle = ink;
    ctx.lineWidth = s.width;
    ctx.beginPath();
    ctx.ellipse((x0 + x1) / 2, (y0 + y1) / 2, (x1 - x0) / 2, (y1 - y0) / 2, 0, 0, Math.PI * 2);
    ctx.stroke();
  } else if (s.kind === 'text') {
    ctx.fillStyle = ink;
    ctx.font = `${s.size}px ${FONT}`;
    ctx.textBaseline = 'top';
    ctx.fillText(s.text, s.at[0], s.at[1]);
  } else if (s.kind === 'number') {
    // The digits centred: across by the middle, down by half a digit's height (0.716
    // of the size in Arial), as markup.rs does.
    ctx.fillStyle = ink;
    ctx.beginPath();
    ctx.arc(s.at[0], s.at[1], s.size * 0.8, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = light(ink) ? '#000000' : '#ffffff';
    ctx.font = `${s.size}px ${FONT}`;
    ctx.textAlign = 'center';
    ctx.textBaseline = 'alphabetic';
    ctx.fillText(String(s.n), s.at[0], s.at[1] + s.size * 0.358);
    ctx.textAlign = 'start';
  } else if (s.kind === 'redact') {
    ctx.fillStyle = '#000000';
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  }
}

// The box a mark takes up, in the picture's pixels: [left, top, right, bottom].
function bounds(s) {
  if (s.kind === 'text') {
    measure.font = `${s.size}px ${FONT}`;
    return [s.at[0], s.at[1], s.at[0] + measure.measureText(s.text).width, s.at[1] + s.size * 1.15];
  }
  if (s.kind === 'number') {
    const r = s.size * 0.8;
    return [s.at[0] - r, s.at[1] - r, s.at[0] + r, s.at[1] + r];
  }
  const points = s.points ?? [s.from, s.to];
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  const pad = (s.width ?? 0) / 2 + (s.kind === 'arrow' ? Math.max(s.width * 4, 12) / 2 : 0);
  return [Math.min(...xs) - pad, Math.min(...ys) - pad, Math.max(...xs) + pad, Math.max(...ys) + pad];
}

function fromSegment(p, a, b) {
  const [dx, dy] = [b[0] - a[0], b[1] - a[1]];
  const t = dx || dy ? Math.max(0, Math.min(1, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy))) : 0;
  return Math.hypot(p[0] - a[0] - t * dx, p[1] - a[1] - t * dy);
}

// Whether the point `p` is on the mark: on its line, near its outline, or inside it
// for the filled ones and words. `tol` is the slack, in the picture's pixels.
function hits(s, p, tol) {
  const reach = tol + (s.width ?? 0) / 2;
  const [x0, y0, x1, y1] = s.from ? corners(s.from, s.to) : [];
  switch (s.kind) {
    case 'pen':
    case 'highlight':
      return s.points.length === 1
        ? Math.hypot(p[0] - s.points[0][0], p[1] - s.points[0][1]) <= reach
        : s.points.some((q, i) => i > 0 && fromSegment(p, s.points[i - 1], q) <= reach);
    case 'arrow':
    case 'line':
      return fromSegment(p, s.from, s.to) <= reach;
    case 'box': {
      const outer = p[0] >= x0 - reach && p[0] <= x1 + reach && p[1] >= y0 - reach && p[1] <= y1 + reach;
      const inner = p[0] > x0 + reach && p[0] < x1 - reach && p[1] > y0 + reach && p[1] < y1 - reach;
      return outer && !inner;
    }
    case 'circle': {
      const [cx, cy] = [(x0 + x1) / 2, (y0 + y1) / 2];
      const [rx, ry] = [Math.max((x1 - x0) / 2, 1), Math.max((y1 - y0) / 2, 1)];
      return Math.abs(Math.hypot((p[0] - cx) / rx, (p[1] - cy) / ry) - 1) * Math.min(rx, ry) <= reach;
    }
    case 'redact':
      return p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1;
    default: {
      const [a, b, c, d] = bounds(s);
      return p[0] >= a - tol && p[0] <= c + tol && p[1] >= b - tol && p[1] <= d + tol;
    }
  }
}

// The mark under the point, the top one first (blacked-out parts are on top of all),
// or -1. With `kind`, only marks of that kind.
function shapeAt(p, kind) {
  const tol = 6 / view.scale;
  const order = shapes.map((_, i) => i).reverse().filter((i) => !kind || shapes[i].kind === kind);
  const top = [...order.filter((i) => shapes[i].kind === 'redact'), ...order.filter((i) => shapes[i].kind !== 'redact')];
  return top.find((i) => hits(shapes[i], p, tol)) ?? -1;
}

// The mark moved by dx, dy, kept inside the picture as the app wants it.
function moved(s, dx, dy) {
  const points = s.points ?? (s.from ? [s.from, s.to] : [s.at]);
  const xs = points.map((p) => p[0]);
  const ys = points.map((p) => p[1]);
  const mx = Math.min(Math.max(dx, -Math.min(...xs)), session.width - Math.max(...xs));
  const my = Math.min(Math.max(dy, -Math.min(...ys)), session.height - Math.max(...ys));
  const shift = (p) => [p[0] + mx, p[1] + my];
  const out = { ...s };
  if (s.points) out.points = s.points.map(shift);
  if (s.from) {
    out.from = shift(s.from);
    out.to = shift(s.to);
  }
  if (s.at) out.at = shift(s.at);
  return out;
}

function redraw() {
  if (!session) return;
  const r = stage.getBoundingClientRect();
  ctx.setTransform(view.dpr, 0, 0, view.dpr, 0, 0);
  ctx.clearRect(0, 0, r.width, r.height);
  ctx.drawImage(image, view.left, view.top, session.width * view.scale, session.height * view.scale);
  ctx.save();
  ctx.translate(view.left, view.top);
  ctx.scale(view.scale, view.scale);
  // Words being retyped show in their box instead.
  const retyped = typing ? shapes[typing.index] : null;
  const all = (drawing ? [...shapes, drawing] : shapes).filter((s) => s !== retyped);
  for (const s of all) if (s.kind !== 'redact') paint(s);
  for (const s of all) if (s.kind === 'redact') paint(s);
  ctx.restore();
  const picked = shapes[selected];
  if (picked && picked !== retyped) {
    const [a, b, c, d] = bounds(picked);
    const x = Math.round(view.left + a * view.scale) - 4.5;
    const y = Math.round(view.top + b * view.scale) - 4.5;
    const w = Math.round((c - a) * view.scale) + 9;
    const h = Math.round((d - b) * view.scale) + 9;
    ctx.lineWidth = 3;
    ctx.strokeStyle = 'rgba(0, 0, 0, 0.45)';
    ctx.strokeRect(x, y, w, h);
    ctx.lineWidth = 1;
    ctx.setLineDash([5, 4]);
    ctx.strokeStyle = '#ffffff';
    ctx.strokeRect(x, y, w, h);
    ctx.setLineDash([]);
  }
  apply.disabled = shapes.length === 0;
  undo.disabled = history.length === 0 && !typing;
}

function press(group, attribute, value) {
  for (const button of group.querySelectorAll(`[${attribute}]`)) {
    button.setAttribute('aria-pressed', String(button.getAttribute(attribute) === value));
  }
}

// Picks the mark (or none with -1): the colour and size boxes show its own.
function select(i) {
  selected = i;
  const s = shapes[i];
  if (s?.colour) useColour(s.colour);
  const kind = s && SIZE_OF[s.kind];
  if (kind) sizes[kind] = toPoints(s[sizeField(s)], kind);
  showSize();
  redraw();
}

// Words: a small box on the picture where they will land, in their colour and size.
// With `index`, the words already there are retyped in it.
function openTyping(at, index = -1) {
  const old = shapes[index];
  const box = document.createElement('input');
  box.className = 'typing';
  box.maxLength = MAX_WORDS;
  box.autocomplete = 'off';
  box.spellcheck = false;
  box.value = old ? old.text : '';
  box.setAttribute('aria-label', 'Words to put on the picture');
  box.addEventListener('input', fitTyping);
  box.addEventListener('keydown', (event) => {
    event.stopPropagation();
    if (event.key === 'Enter') {
      event.preventDefault();
      placeTyping();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      dropTyping();
    }
  });
  box.addEventListener('blur', (event) => {
    // Into the size box or the palette keeps it open; anywhere else puts the words on.
    if (event.relatedTarget?.closest('footer, .pop')) return;
    placeTyping();
  });
  if (old) {
    useColour(old.colour);
    sizes.text = toPoints(old.size, 'text');
  }
  stage.append(box);
  typing = { box, at, index: old ? index : -1, size: old ? old.size : sizeNow('text') };
  showSize();
  redraw();
  requestAnimationFrame(() => {
    if (typing?.box !== box) return;
    box.focus();
    box.setSelectionRange(box.value.length, box.value.length);
  });
}

function styleTyping() {
  if (!typing) return;
  const { box, at } = typing;
  const px = typing.size * view.scale;
  box.style.font = `${px}px ${FONT}`;
  box.style.lineHeight = `${px * 1.2}px`;
  box.style.height = `${px * 1.2}px`;
  box.style.color = hexOf(colour);
  // The canvas puts the top of the letters at `at`; the box centres them in its line.
  box.style.left = `${view.left + at[0] * view.scale}px`;
  box.style.top = `${view.top + at[1] * view.scale - px * 0.16}px`;
  typing.px = px;
  fitTyping();
}

function fitTyping() {
  if (!typing) return;
  const { box, px } = typing;
  measure.font = `${px}px ${FONT}`;
  const room = stage.clientWidth - box.offsetLeft - 8;
  const width = measure.measureText(box.value || 'MM').width + 4;
  box.style.width = `${Math.max(16, Math.min(room, width))}px`;
}

function placeTyping() {
  if (!typing) return;
  const { box, at, index, size } = typing;
  typing = null;
  // One line of plain words: anything else would be refused by the app.
  const text = box.value.replace(/\p{Cc}/gu, ' ').trim();
  box.remove();
  const old = shapes[index];
  if (old) {
    if (text !== old.text || colour !== old.colour || size !== old.size) {
      remember();
      if (text) {
        Object.assign(old, { text, colour, size });
      } else {
        // Retyped to nothing: the words go.
        shapes.splice(index, 1);
        selected = -1;
      }
    }
  } else if (text) {
    remember();
    shapes.push({ kind: 'text', colour, size, at, text });
  }
  showSize();
  redraw();
}

function dropTyping() {
  if (!typing) return;
  const { box } = typing;
  typing = null;
  box.remove();
  showSize();
  redraw();
}

canvas.addEventListener('pointerdown', (event) => {
  if (!session || event.button !== 0) return;
  say('');
  // A click away from words being typed puts them on, and does nothing more.
  if (typing) {
    placeTyping();
    return;
  }
  // Leaving the size box takes what was typed in it first.
  if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
  const p = toPicture(event);
  if (tool === 'select') {
    const i = shapeAt(p);
    select(i);
    if (i >= 0) {
      canvas.setPointerCapture(event.pointerId);
      moving = { from: p, original: JSON.parse(JSON.stringify(shapes[i])), moved: false };
    }
    return;
  }
  if (tool === 'text') {
    // A click on words already there retypes them; anywhere else starts new ones.
    const i = shapeAt(p, 'text');
    if (i >= 0) openTyping(shapes[i].at, i);
    else openTyping(p);
    return;
  }
  if (tool === 'number') {
    const n = Math.max(0, ...shapes.filter((s) => s.kind === 'number').map((s) => s.n)) + 1;
    if (n > MAX_STEP) {
      say(`Up to ${MAX_STEP} steps.`);
      return;
    }
    remember();
    shapes.push({ kind: 'number', colour, size: sizeNow('number'), at: p, n });
    redraw();
    return;
  }
  canvas.setPointerCapture(event.pointerId);
  if (tool === 'pen' || tool === 'highlight') drawing = { kind: tool, colour, width: sizeNow(SIZE_OF[tool]), points: [p] };
  else if (tool === 'redact') drawing = { kind: 'redact', from: p, to: p };
  else drawing = { kind: tool, colour, width: sizeNow('line'), from: p, to: p };
  redraw();
});
// The click that opens a words box must not take the focus straight back off it.
canvas.addEventListener('mousedown', (event) => {
  if (tool === 'text' || tool === 'select') event.preventDefault();
});
// Select: a double-click on words retypes them.
canvas.addEventListener('dblclick', (event) => {
  if (!session || tool !== 'select' || typing) return;
  const i = shapeAt(toPicture(event), 'text');
  if (i >= 0) openTyping(shapes[i].at, i);
});

canvas.addEventListener('pointermove', (event) => {
  if (moving) {
    const p = toPicture(event);
    const [dx, dy] = [p[0] - moving.from[0], p[1] - moving.from[1]];
    if (!moving.moved && (dx || dy)) {
      moving.moved = true;
      remember();
    }
    if (moving.moved) {
      shapes[selected] = moved(moving.original, dx, dy);
      redraw();
    }
    return;
  }
  if (!drawing) return;
  const p = toPicture(event);
  if (drawing.points) {
    const last = drawing.points[drawing.points.length - 1];
    // A new point every two screen pixels or so is smooth enough, and keeps it short.
    const far = Math.hypot(p[0] - last[0], p[1] - last[1]) * view.scale >= 2;
    if (far && drawing.points.length < MAX_POINTS) drawing.points.push(p);
  } else {
    drawing.to = p;
  }
  redraw();
});

function finishDrawing() {
  moving = null;
  if (!drawing) return;
  // A click without a drag makes a dot with the pens, and nothing with the others. A
  // black box needs both a width and a height (for a video, the app refuses one without).
  const [dx, dy] = drawing.points ? [1, 1] : [drawing.from[0] !== drawing.to[0], drawing.from[1] !== drawing.to[1]];
  const sized = drawing.kind === 'redact' ? dx && dy : dx || dy;
  if (sized) {
    remember();
    shapes.push(drawing);
  }
  drawing = null;
  redraw();
}
canvas.addEventListener('pointerup', finishDrawing);
canvas.addEventListener('pointercancel', finishDrawing);

// After a colour or a size is picked, typing carries on where it was.
function backToTyping() {
  if (typing) typing.box.focus();
  else if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
}

// The size list, the shapes and the palette open above the bar.
function openPop(pop, anchor, toggle) {
  closePops();
  opener = toggle;
  pop.hidden = false;
  toggle.setAttribute('aria-expanded', 'true');
  const a = anchor.getBoundingClientRect();
  pop.style.left = `${Math.max(8, Math.min(a.left, window.innerWidth - pop.offsetWidth - 8))}px`;
  pop.style.bottom = `${window.innerHeight - a.top + 6}px`;
}

// Closes whichever is open, and says whether one was.
function closePops() {
  const open = POPS.some((pop) => !pop.hidden);
  const inside = document.activeElement?.closest('.pop');
  for (const pop of POPS) pop.hidden = true;
  for (const toggle of [sizeOpen, shapeOpen, more]) toggle.setAttribute('aria-expanded', 'false');
  if (inside) {
    if (typing) typing.box.focus();
    else opener?.focus();
  }
  return open;
}

// Up and Down go through a list's buttons, Escape closes it.
function listKeys(pop) {
  pop.addEventListener('keydown', (event) => {
    event.stopPropagation();
    const options = [...pop.children];
    const at = options.indexOf(document.activeElement);
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      options[Math.min(options.length - 1, Math.max(0, at + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      closePops();
    }
  });
}

function showSize() {
  const kind = sizeKind();
  sizeValue.value = kind ? String(sizes[kind]) : '';
  sizeValue.disabled = !kind;
  sizeOpen.disabled = !kind;
  sizeValue.setAttribute('aria-label', kind ? `Size of the ${NAMES[kind]}, in points` : 'Size');
  styleTyping();
}

// A new size: for what comes next, and for the words being typed or the picked mark.
function setSize(value) {
  const kind = sizeKind();
  if (!kind) {
    showSize();
    return;
  }
  const [low, high] = LIMITS[kind];
  const n = Math.round(Number(value));
  const before = sizes[kind];
  if (Number.isFinite(n) && n > 0) sizes[kind] = Math.min(high, Math.max(low, n));
  if (sizes[kind] !== before) {
    const s = !typing && tool === 'select' ? shapes[selected] : null;
    if (typing) typing.size = sizeNow('text');
    else if (s) {
      remember();
      s[sizeField(s)] = sizeNow(kind);
      redraw();
    }
  }
  showSize();
}

// The next size up or down the list from the one in use.
function stepSize(up) {
  const kind = sizeKind();
  if (!kind) return;
  const list = SIZES[kind];
  const now = sizes[kind];
  const next = up ? list.find((v) => v > now) : [...list].reverse().find((v) => v < now);
  if (next !== undefined) setSize(next);
}

function openSizes() {
  const kind = sizeKind();
  if (!kind) return;
  sizeList.replaceChildren(...SIZES[kind].map((value) => {
    const option = document.createElement('button');
    option.type = 'button';
    option.textContent = String(value);
    option.setAttribute('role', 'option');
    option.setAttribute('aria-selected', String(value === sizes[kind]));
    option.addEventListener('click', () => {
      setSize(value);
      closePops();
    });
    return option;
  }));
  openPop(sizeList, sizeOpen.parentElement, sizeOpen);
  (sizeList.querySelector('[aria-selected="true"]') ?? sizeList.firstElementChild).focus();
}

sizeValue.addEventListener('change', () => setSize(sizeValue.value));
sizeValue.addEventListener('keydown', (event) => {
  if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
    event.preventDefault();
    event.stopPropagation();
    setSize(sizeValue.value);
    stepSize(event.key === 'ArrowUp');
  } else if (event.key === 'Enter' || event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Enter') setSize(sizeValue.value);
    else showSize();
    backToTyping();
  }
});
sizeOpen.addEventListener('click', () => (sizeList.hidden ? openSizes() : closePops()));
listKeys(sizeList);

function openShapes() {
  for (const item of shapeList.children) item.setAttribute('aria-current', String(item.dataset.shape === shapeTool.dataset.tool));
  openPop(shapeList, shapeTool, shapeOpen);
  (shapeList.querySelector('[aria-current="true"]') ?? shapeList.firstElementChild).focus();
}
shapeOpen.addEventListener('click', () => (shapeList.hidden ? openShapes() : closePops()));
for (const item of shapeList.children) item.addEventListener('click', () => pickTool(item.dataset.shape));
listKeys(shapeList);

function swatch(value, label) {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'swatch';
  button.style.background = hexOf(value);
  button.setAttribute('aria-label', label);
  return button;
}

const more = swatch('#ffffff', 'More colours');
more.classList.add('more');
more.style.background = '';
more.setAttribute('aria-haspopup', 'dialog');
more.setAttribute('aria-expanded', 'false');
more.setAttribute('aria-controls', 'palette');
more.addEventListener('click', () => (palette.hidden ? openPalette() : closePops()));

// The quick colours, the ones picked lately from the palette, then More.
function showColours() {
  const row = [...Object.keys(COLOURS), ...recent].map((name) => {
    const button = swatch(name, name);
    button.dataset.colour = name;
    button.addEventListener('click', () => pickColour(name));
    return button;
  });
  colours.replaceChildren(...row, more);
  press(colours, 'data-colour', colour);
}

// The colour for what comes next, shown pressed (a new code joins the row).
function useColour(value) {
  colour = value;
  if (!COLOURS[colour] && !recent.includes(colour)) {
    recent = [colour, ...recent].slice(0, RECENT);
    showColours();
  }
  press(colours, 'data-colour', colour);
}

// A picked colour: a quick one goes by its name, any other by its code. It also
// recolours the words being typed or the picked mark.
function pickColour(value) {
  useColour(Object.keys(COLOURS).find((name) => COLOURS[name] === value) ?? value);
  const s = !typing && tool === 'select' ? shapes[selected] : null;
  if (s?.colour && s.colour !== colour) {
    remember();
    s.colour = colour;
    redraw();
  }
  styleTyping();
}

function openPalette() {
  code.value = hexOf(colour);
  code.removeAttribute('aria-invalid');
  openPop(palette, more, more);
  (grid.querySelector(`[data-pick="${hexOf(colour)}"]`) ?? grid.firstElementChild).focus();
}

for (const value of PALETTE) {
  const button = swatch(value, value);
  button.dataset.pick = value;
  button.addEventListener('click', () => {
    pickColour(value);
    closePops();
  });
  grid.append(button);
}
palette.addEventListener('keydown', (event) => {
  event.stopPropagation();
  if (event.key === 'Escape') {
    event.preventDefault();
    closePops();
    return;
  }
  const cells = [...grid.children];
  const at = cells.indexOf(document.activeElement);
  const move = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -10, ArrowDown: 10 }[event.key];
  if (at >= 0 && move) {
    event.preventDefault();
    cells[at + move]?.focus();
  }
});
code.addEventListener('input', () => code.removeAttribute('aria-invalid'));
code.addEventListener('keydown', (event) => {
  if (event.key !== 'Enter') return;
  event.preventDefault();
  const typed = code.value.trim().toLowerCase().replace(/^#?/, '#');
  if (/^#[0-9a-f]{6}$/.test(typed)) {
    say('');
    pickColour(typed);
    closePops();
  } else {
    code.setAttribute('aria-invalid', 'true');
    say('A colour code is # and six of 0-9 or a-f, like #ff8800.');
  }
});

// A click outside a list or the palette closes it.
document.addEventListener('pointerdown', (event) => {
  if (!event.target.closest?.('.pop, #sizeOpen, #shapeOpen, .more')) closePops();
}, true);
window.addEventListener('resize', () => closePops());
// While words are being typed, clicking a button keeps the typing going.
for (const area of [document.querySelector('footer'), ...POPS]) {
  area.addEventListener('mousedown', (event) => {
    if (typing && event.target.closest('button')) event.preventDefault();
  });
}

async function save() {
  placeTyping();
  if (!session || apply.disabled) return;
  apply.disabled = true;
  try {
    // The app checks the shapes, queues the job and closes this window.
    await invoke(session.video ? 'editor_blackout' : 'editor_markup', { markup: { shapes } });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

function undoLast() {
  if (typing) {
    dropTyping();
    return;
  }
  if (!history.length) return;
  shapes = JSON.parse(history.pop());
  selected = -1;
  showSize();
  redraw();
}

function removePicked() {
  if (!shapes[selected]) return;
  remember();
  shapes.splice(selected, 1);
  select(-1);
}

function pickTool(name) {
  placeTyping();
  closePops();
  if (SHAPES[name]) {
    shapeTool.dataset.tool = name;
    shapeTool.textContent = SHAPES[name];
  }
  tool = name;
  if (tool !== 'select' && selected >= 0) select(-1);
  press(tools, 'data-tool', tool);
  canvas.style.cursor = { text: 'text', select: 'default' }[tool] ?? 'crosshair';
  showSize();
}

for (const button of tools.querySelectorAll('[data-tool]')) {
  button.addEventListener('click', () => pickTool(button.dataset.tool));
}
undo.addEventListener('click', undoLast);
apply.addEventListener('click', save);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  const inField = document.activeElement instanceof HTMLInputElement;
  if (event.key === 'Escape') {
    if (closePops()) return;
    if (selected >= 0) select(-1);
    else cancel();
  } else if (inField) {
    // Typing in a box: its own keys.
  } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z') {
    event.preventDefault();
    undoLast();
  } else if ((event.key === 'Delete' || event.key === 'Backspace') && selected >= 0) {
    event.preventDefault();
    removePicked();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    save();
  } else if (event.key.startsWith('Arrow')) {
    const step = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
    if (!step) return;
    event.preventDefault();
    if (shapes[selected]) {
      // A picked mark moves: a screen pixel at a time, ten with Shift.
      const by = Math.max(1, Math.round((event.shiftKey ? 10 : 1) / view.scale));
      remember();
      shapes[selected] = moved(shapes[selected], step[0] * by, step[1] * by);
      redraw();
    } else if (step[0]) {
      // Otherwise Left and Right pick the tool, Up and Down the colour.
      const next = toolOrder[toolOrder.indexOf(tool) + step[0]];
      if (next) pickTool(next);
    } else if (!session?.video) {
      const list = [...colours.querySelectorAll('[data-colour]')];
      const next = list[list.findIndex((button) => button.dataset.colour === colour) + step[1]];
      if (next) pickColour(next.dataset.colour);
    }
  }
});

showColours();
pickTool('pen');
new ResizeObserver(layout).observe(stage);

// Black Out on a video: only Select and Black out, no colours or sizes (the boxes are
// black, and fill what they cover).
function videoOnly() {
  toolOrder = VIDEO_TOOLS;
  for (const button of tools.querySelectorAll('[data-tool]')) {
    if (!VIDEO_TOOLS.includes(button.dataset.tool)) button.hidden = true;
  }
  // Both have a display of their own, which [hidden] wouldn't override.
  shapeTool.closest('.split').style.display = 'none';
  colours.closest('.row').style.display = 'none';
  pickTool('redact');
  say('Draw a box over each part to hide. It stays black for the whole video.');
}

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session
    && session.kind === 'markup'
    && Number.isSafeInteger(session.width) && session.width > 0
    && Number.isSafeInteger(session.height) && session.height > 0
    && typeof session.preview === 'string' && session.preview.startsWith('data:image/jpeg;base64,')
    && typeof session.video === 'boolean';
  if (!valid) {
    session = null;
    say('Nothing to mark up. Drag the picture onto Mark Up again.');
    return;
  }
  if (session.video) videoOnly();
  document.title = `${session.video ? 'Black out' : 'Mark up'}: ${session.name}`;
  image.onload = layout;
  image.src = session.preview;
}

load();
