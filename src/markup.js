// The Mark Up window: the picture, and tools to draw on it (pen, arrow, box, words) or
// black parts of it out, in a few colours, with Undo. What is drawn is kept as shapes
// in the picture's own pixels; the app checks them and ImageMagick draws them the same
// way (src-tauri/src/convert/markup.rs), the blacked-out parts last, over everything.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const stage = $('stage');
const canvas = $('photo');
const ctx = canvas.getContext('2d');
const words = $('words');
const apply = $('apply');
const undo = $('undo');
const message = $('message');

// The same colours as COLOURS in markup.rs, by the names sent back.
const COLOURS = { red: '#e5322d', yellow: '#f5c400', blue: '#1d6fe5', green: '#21a148', black: '#000000', white: '#ffffff' };
const MAX_POINTS = 4000;

// { name, width, height, preview } from the app, once loaded.
let session = null;
const image = new Image();
let shapes = [];
let drawing = null;
let tool = 'pen';
let colour = 'red';
let thick = false;
// Where the picture sits on the canvas: picture pixels times `scale`, from left, top.
let view = { scale: 1, left: 0, top: 0, dpr: 1 };

function say(text) {
  message.textContent = text;
  message.hidden = !text;
}

// Line and word sizes in the picture's pixels, so they look the same on any picture.
function lineWidth() {
  const base = Math.max(2, Math.round(Math.min(session.width, session.height) / 250));
  return Math.min(200, thick ? base * 3 : base);
}

function textSize() {
  return Math.min(1000, Math.max(14, Math.round(Math.min(session.width, session.height) / 20)));
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
    ctx.strokeStyle = COLOURS[s.colour];
    ctx.fillStyle = COLOURS[s.colour];
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
    ctx.strokeStyle = COLOURS[s.colour];
    ctx.fillStyle = COLOURS[s.colour];
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
    ctx.strokeStyle = COLOURS[s.colour];
    ctx.lineWidth = s.width;
    ctx.strokeRect(x0, y0, x1 - x0, y1 - y0);
  } else if (s.kind === 'text') {
    ctx.fillStyle = COLOURS[s.colour];
    ctx.font = `${s.size}px Arial, Helvetica, "DejaVu Sans", sans-serif`;
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

canvas.addEventListener('pointerdown', (event) => {
  if (!session || event.button !== 0) return;
  const p = toPicture(event);
  say('');
  if (tool === 'text') {
    const text = words.value.trim();
    if (!text) {
      say('Type the words first, then click where they go.');
      words.focus();
      return;
    }
    shapes.push({ kind: 'text', colour, size: textSize(), at: p, text });
    redraw();
    return;
  }
  canvas.setPointerCapture(event.pointerId);
  if (tool === 'pen') drawing = { kind: 'pen', colour, width: lineWidth(), points: [p] };
  else if (tool === 'redact') drawing = { kind: 'redact', from: p, to: p };
  else drawing = { kind: tool, colour, width: lineWidth(), from: p, to: p };
  redraw();
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

async function save() {
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
  shapes.pop();
  redraw();
}

for (const button of document.querySelectorAll('[data-tool]')) {
  button.addEventListener('click', () => {
    tool = button.dataset.tool;
    press(button.closest('.group'), 'data-tool', tool);
    if (tool === 'text') words.focus();
  });
}
for (const button of document.querySelectorAll('[data-thick]')) {
  button.addEventListener('click', () => {
    thick = button.dataset.thick === 'true';
    press(button.closest('.group'), 'data-thick', String(thick));
  });
}
for (const [name, hex] of Object.entries(COLOURS)) {
  const swatch = document.createElement('button');
  swatch.type = 'button';
  swatch.className = 'swatch';
  swatch.dataset.colour = name;
  swatch.style.background = hex;
  swatch.setAttribute('aria-label', name);
  swatch.setAttribute('aria-pressed', String(name === colour));
  swatch.addEventListener('click', () => {
    colour = name;
    press($('colours'), 'data-colour', name);
  });
  $('colours').append(swatch);
}
undo.addEventListener('click', undoLast);
apply.addEventListener('click', save);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'z' && document.activeElement !== words) {
    event.preventDefault();
    undoLast();
  } else if (event.key === 'Enter' && document.activeElement !== words && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    save();
  }
});

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
