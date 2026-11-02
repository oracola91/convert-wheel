// The Mark Up window: the picture, and tools to draw on it (pen, arrow, box, words) or
// black parts of it out, with Undo. Sizes work like Word's, in points as the picture
// looks in this window; colours are six quick ones, or any from the palette or typed
// as a code. Words are typed straight onto the picture: click where they go, type,
// Enter. What is drawn is kept as shapes in the picture's own pixels; the app checks
// them and ImageMagick draws them the same way (src-tauri/src/convert/markup.rs), the
// blacked-out parts last, over everything.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
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
const palette = $('palette');
const grid = $('grid');
const code = $('code');
const tools = document.querySelector('[data-tool]').closest('.group');

// The quick colours, the same as COLOURS in markup.rs, by the names sent back. Any
// other colour is sent as its #rrggbb code.
const COLOURS = { red: '#e5322d', yellow: '#f5c400', blue: '#1d6fe5', green: '#21a148', black: '#000000', white: '#ffffff' };
const hexOf = (c) => COLOURS[c] ?? c;
const FONT = 'Arial, Helvetica, "DejaVu Sans", sans-serif';
const MAX_POINTS = 4000;
const MAX_WORDS = 200;
// Palette colours picked lately, kept next to the quick ones.
const RECENT = 3;

// The sizes to pick, as in Word: font sizes for words, line weights for the rest. Any
// other whole number can be typed, within LIMITS.
const SIZES = {
  text: [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72],
  line: [1, 2, 3, 4, 6, 8, 10, 12, 16, 20],
};
const LIMITS = { text: [1, 400], line: [1, 100] };
const sizes = { text: 24, line: 3 };

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
let drawing = null;
// The words being typed, if any: { box, at } with `at` in the picture's pixels.
let typing = null;
let tool = 'pen';
let colour = 'red';
let recent = [];
// The button that opened the size list or the palette, to go back to.
let opener = null;
// Where the picture sits on the canvas: picture pixels times `scale`, from left, top.
let view = { scale: 1, left: 0, top: 0, dpr: 1 };

function say(text) {
  message.textContent = text;
  message.hidden = !text;
}

const kind = () => (tool === 'text' ? 'text' : 'line');

// A size in points as the picture looks in this window (a point is 4/3 of a screen
// pixel), in the picture's own pixels and within what the app takes. What is seen is
// what is saved, whatever the picture's size.
function pixels(points, min, max) {
  return Math.min(max, Math.max(min, Math.round((points * 4) / 3 / view.scale)));
}
const lineWidth = () => pixels(sizes.line, 1, 200);
const textSize = () => pixels(sizes.text, 8, 1000);

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
  const [x0, y0, x1, y1] = s.from ? [Math.min(s.from[0], s.to[0]), Math.min(s.from[1], s.to[1]), Math.max(s.from[0], s.to[0]), Math.max(s.from[1], s.to[1])] : [];
  if (s.kind === 'pen') {
    ctx.strokeStyle = hexOf(s.colour);
    ctx.fillStyle = hexOf(s.colour);
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
    ctx.strokeStyle = hexOf(s.colour);
    ctx.fillStyle = hexOf(s.colour);
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
  } else if (s.kind === 'box') {
    ctx.strokeStyle = hexOf(s.colour);
    ctx.lineWidth = s.width;
    ctx.strokeRect(x0, y0, x1 - x0, y1 - y0);
  } else if (s.kind === 'text') {
    ctx.fillStyle = hexOf(s.colour);
    ctx.font = `${s.size}px ${FONT}`;
    ctx.textBaseline = 'top';
    ctx.fillText(s.text, s.at[0], s.at[1]);
  } else if (s.kind === 'redact') {
    ctx.fillStyle = '#000000';
    ctx.fillRect(x0, y0, x1 - x0, y1 - y0);
  }
}

function redraw() {
  const r = stage.getBoundingClientRect();
  ctx.setTransform(view.dpr, 0, 0, view.dpr, 0, 0);
  ctx.clearRect(0, 0, r.width, r.height);
  ctx.drawImage(image, view.left, view.top, session.width * view.scale, session.height * view.scale);
  ctx.save();
  ctx.translate(view.left, view.top);
  ctx.scale(view.scale, view.scale);
  const all = drawing ? [...shapes, drawing] : shapes;
  for (const s of all) if (s.kind !== 'redact') paint(s);
  for (const s of all) if (s.kind === 'redact') paint(s);
  ctx.restore();
  apply.disabled = shapes.length === 0;
  undo.disabled = shapes.length === 0;
}

function press(group, attribute, value) {
  for (const button of group.querySelectorAll(`[${attribute}]`)) {
    button.setAttribute('aria-pressed', String(button.getAttribute(attribute) === value));
  }
}

// Words: a small box on the picture where they will land, in their colour and size.
function openTyping(at) {
  const box = document.createElement('input');
  box.className = 'typing';
  box.maxLength = MAX_WORDS;
  box.autocomplete = 'off';
  box.spellcheck = false;
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
  stage.append(box);
  typing = { box, at };
  styleTyping();
  requestAnimationFrame(() => typing?.box.focus());
}

function styleTyping() {
  if (!typing) return;
  const { box, at } = typing;
  const px = textSize() * view.scale;
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
  const { box, at } = typing;
  typing = null;
  // One line of plain words: anything else would be refused by the app.
  const text = box.value.replace(/\p{Cc}/gu, ' ').trim();
  box.remove();
  if (text) shapes.push({ kind: 'text', colour, size: textSize(), at, text });
  redraw();
}

function dropTyping() {
  if (!typing) return;
  const { box } = typing;
  typing = null;
  box.remove();
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
  const p = toPicture(event);
  if (tool === 'text') {
    openTyping(p);
    return;
  }
  canvas.setPointerCapture(event.pointerId);
  if (tool === 'pen') drawing = { kind: 'pen', colour, width: lineWidth(), points: [p] };
  else if (tool === 'redact') drawing = { kind: 'redact', from: p, to: p };
  else drawing = { kind: tool, colour, width: lineWidth(), from: p, to: p };
  redraw();
});
// The click that opens a words box must not take the focus straight back off it.
canvas.addEventListener('mousedown', (event) => {
  if (tool === 'text') event.preventDefault();
});

canvas.addEventListener('pointermove', (event) => {
  if (!drawing) return;
  const p = toPicture(event);
  if (drawing.kind === 'pen') {
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
  if (!drawing) return;
  // A click without a drag makes a dot with the pen, and nothing with the others.
  const sized = drawing.kind === 'pen' || drawing.from[0] !== drawing.to[0] || drawing.from[1] !== drawing.to[1];
  if (sized) shapes.push(drawing);
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

// The size list and the palette, above the bar.
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
  const open = !sizeList.hidden || !palette.hidden;
  const inside = document.activeElement?.closest('.pop');
  sizeList.hidden = true;
  palette.hidden = true;
  sizeOpen.setAttribute('aria-expanded', 'false');
  more.setAttribute('aria-expanded', 'false');
  if (inside) {
    if (typing) typing.box.focus();
    else opener?.focus();
  }
  return open;
}

function showSize() {
  const off = tool === 'redact';
  sizeValue.value = off ? '' : String(sizes[kind()]);
  sizeValue.disabled = off;
  sizeOpen.disabled = off;
  sizeValue.setAttribute('aria-label', tool === 'text' ? 'Size of the words, in points' : 'Size of the line, in points');
  styleTyping();
}

function setSize(value) {
  const [low, high] = LIMITS[kind()];
  const n = Math.round(Number(value));
  if (Number.isFinite(n) && n > 0) sizes[kind()] = Math.min(high, Math.max(low, n));
  showSize();
}

// The next size up or down the list from the one in use.
function stepSize(up) {
  const list = SIZES[kind()];
  const now = sizes[kind()];
  const next = up ? list.find((v) => v > now) : [...list].reverse().find((v) => v < now);
  if (next !== undefined) setSize(next);
}

function openSizes() {
  const now = sizes[kind()];
  sizeList.replaceChildren(...SIZES[kind()].map((value) => {
    const option = document.createElement('button');
    option.type = 'button';
    option.textContent = String(value);
    option.setAttribute('role', 'option');
    option.setAttribute('aria-selected', String(value === now));
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
sizeList.addEventListener('keydown', (event) => {
  event.stopPropagation();
  const options = [...sizeList.children];
  const at = options.indexOf(document.activeElement);
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    options[Math.min(options.length - 1, Math.max(0, at + (event.key === 'ArrowDown' ? 1 : -1)))]?.focus();
  } else if (event.key === 'Escape') {
    event.preventDefault();
    closePops();
  }
});

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

// A quick colour goes by its name, any other by its code.
function pickColour(value) {
  colour = Object.keys(COLOURS).find((name) => COLOURS[name] === value) ?? value;
  if (!COLOURS[colour] && !recent.includes(colour)) {
    recent = [colour, ...recent].slice(0, RECENT);
    showColours();
  }
  press(colours, 'data-colour', colour);
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

// A click outside the size list or the palette closes it.
document.addEventListener('pointerdown', (event) => {
  if (!event.target.closest?.('.pop, #sizeOpen, .more')) closePops();
}, true);
window.addEventListener('resize', () => closePops());
// While words are being typed, clicking a button keeps the typing going.
for (const area of [document.querySelector('footer'), sizeList, palette]) {
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
    await invoke('editor_markup', { markup: { shapes } });
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
  shapes.pop();
  redraw();
}

function pickTool(name) {
  placeTyping();
  tool = name;
  press(tools, 'data-tool', tool);
  canvas.style.cursor = tool === 'text' ? 'text' : '';
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
    if (!closePops()) cancel();
  } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z' && !inField) {
    event.preventDefault();
    undoLast();
  } else if (event.key === 'Enter' && !inField && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    save();
  } else if (!inField && event.key.startsWith('Arrow')) {
    // Left and Right pick the tool, Up and Down the colour (not while typing).
    const across = event.key === 'ArrowLeft' || event.key === 'ArrowRight';
    const list = across ? [...tools.querySelectorAll('[data-tool]')] : [...colours.querySelectorAll('[data-colour]')];
    const now = list.findIndex((button) => button.getAttribute('aria-pressed') === 'true');
    const next = list[now + (event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : -1)];
    if (next) {
      event.preventDefault();
      if (across) pickTool(next.dataset.tool);
      else pickColour(next.dataset.colour);
    }
  }
});

showColours();
showSize();
new ResizeObserver(layout).observe(stage);

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
    && typeof session.preview === 'string' && session.preview.startsWith('data:image/jpeg;base64,');
  if (!valid) {
    session = null;
    say('Nothing to mark up. Drag the picture onto Mark Up again.');
    return;
  }
  document.title = `Mark up: ${session.name}`;
  image.onload = layout;
  image.src = session.preview;
}

load();
