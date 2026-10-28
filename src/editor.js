// The editor window: the picture, a crop box to drag (or its width and height to type),
// shapes to pick, quarter turns, then Crop or Cancel. The app hands over a preview of
// the picture and its size; this page sends back only the box, in pixels of the full
// picture as turned.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const stage = document.getElementById('stage');
const canvas = document.getElementById('photo');
const box = document.getElementById('box');
const widthIn = document.getElementById('width');
const heightIn = document.getElementById('height');
const message = document.getElementById('message');
const apply = document.getElementById('apply');

// The smallest box, in pixels of the picture, and the space around the picture.
const MIN = 8;
const MARGIN = 24;

const picture = new Image();
// { name, width, height, preview } from the app, once loaded.
let session = null;
// Quarter turns clockwise, 0 to 3.
let turns = 0;
// The width over height the box keeps: null for any, 'original' for the picture's own,
// or a number for the wide way round (16:9 is 1.78).
let shape = null;
// The box, in pixels of the full picture as turned.
let crop = { x: 0, y: 0, w: 1, h: 1 };
// Where the picture sits on the stage, and how many screen pixels one picture pixel takes.
let view = { left: 0, top: 0, scale: 1 };
// The drag under way: what is dragged, the box and the pointer when it started.
let drag = null;

box.hidden = true;

function clamp(v, lo, hi) {
  return Math.min(Math.max(v, lo), hi);
}

function turned() {
  return turns % 2 ? { w: session.height, h: session.width } : { w: session.width, h: session.height };
}

// The ratio to keep now. Preset shapes follow the picture, so 16:9 is wide on a wide
// picture and tall on a tall one.
function ratio() {
  if (shape === null) return null;
  const t = turned();
  if (shape === 'original') return t.w / t.h;
  return t.w >= t.h ? shape : 1 / shape;
}

// Whole pixels, inside the picture.
function snap(b) {
  const t = turned();
  const x = clamp(Math.round(b.x), 0, t.w - 1);
  const y = clamp(Math.round(b.y), 0, t.h - 1);
  return { x, y, w: clamp(Math.round(b.w), 1, t.w - x), h: clamp(Math.round(b.h), 1, t.h - y) };
}

// The largest box of the chosen shape, in the middle.
function fullBox() {
  const t = turned();
  const r = ratio();
  let w = t.w;
  let h = t.h;
  if (r) {
    if (w / h > r) w = h * r;
    else h = w / r;
  }
  return snap({ x: (t.w - w) / 2, y: (t.h - h) / 2, w, h });
}

// A width or height typed in the bar. The box keeps its middle where it can and stays
// inside the picture; with a shape to keep, the other side follows.
function typed(side, value) {
  const t = turned();
  const r = ratio();
  let w = side === 'w' ? value : crop.w;
  let h = side === 'h' ? value : crop.h;
  if (r) {
    if (side === 'w') h = w / r;
    else w = h * r;
    // Too big for the picture: smaller, the same shape.
    const fit = Math.min(1, t.w / w, t.h / h);
    w *= fit;
    h *= fit;
  }
  w = clamp(Math.round(w), 1, t.w);
  h = clamp(Math.round(h), 1, t.h);
  const x = clamp(Math.round(crop.x + crop.w / 2 - w / 2), 0, t.w - w);
  const y = clamp(Math.round(crop.y + crop.h / 2 - h / 2), 0, t.h - h);
  return { x, y, w, h };
}

function moved(s, dx, dy) {
  const t = turned();
  return { x: clamp(Math.round(s.x + dx), 0, t.w - s.w), y: clamp(Math.round(s.y + dy), 0, t.h - s.h), w: s.w, h: s.h };
}

// Dragging a side or a corner. With a shape to keep, the dragged side leads (from a
// corner, the way it moved more), the opposite side or corner stays put, a side keeps
// its middle, and the box shrinks to stay inside the picture.
function resized(s, edge, dx, dy) {
  const t = turned();
  let left = s.x;
  let top = s.y;
  let right = s.x + s.w;
  let bottom = s.y + s.h;
  if (edge.includes('w')) left = clamp(left + dx, 0, right - MIN);
  if (edge.includes('e')) right = clamp(right + dx, left + MIN, t.w);
  if (edge.includes('n')) top = clamp(top + dy, 0, bottom - MIN);
  if (edge.includes('s')) bottom = clamp(bottom + dy, top + MIN, t.h);
  const r = ratio();
  if (!r) return snap({ x: left, y: top, w: right - left, h: bottom - top });
  let w = right - left;
  let h = bottom - top;
  const sideways = edge === 'e' || edge === 'w';
  const upDown = edge === 'n' || edge === 's';
  if (sideways || (!upDown && w / s.w >= h / s.h)) h = w / r;
  else w = h * r;
  const ax = edge.includes('w') ? s.x + s.w : edge.includes('e') ? s.x : s.x + s.w / 2;
  const ay = edge.includes('n') ? s.y + s.h : edge.includes('s') ? s.y : s.y + s.h / 2;
  const roomW = edge.includes('w') ? ax : edge.includes('e') ? t.w - ax : 2 * Math.min(ax, t.w - ax);
  const roomH = edge.includes('n') ? ay : edge.includes('s') ? t.h - ay : 2 * Math.min(ay, t.h - ay);
  const fit = Math.min(1, roomW / w, roomH / h);
  w *= fit;
  h *= fit;
  const x = edge.includes('w') ? ax - w : edge.includes('e') ? ax : ax - w / 2;
  const y = edge.includes('n') ? ay - h : edge.includes('s') ? ay : ay - h / 2;
  return snap({ x, y, w, h });
}

// Draws the picture, turned, as large as the stage allows, sharp on any screen.
function layout() {
  if (!session || !picture.complete) return;
  const t = turned();
  widthIn.title = `Width in pixels, up to ${t.w}`;
  heightIn.title = `Height in pixels, up to ${t.h}`;
  const area = stage.getBoundingClientRect();
  const scale = Math.max(0.01, Math.min((area.width - 2 * MARGIN) / t.w, (area.height - 2 * MARGIN) / t.h));
  const w = t.w * scale;
  const h = t.h * scale;
  view = { left: (area.width - w) / 2, top: (area.height - h) / 2, scale };
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(w * dpr));
  canvas.height = Math.max(1, Math.round(h * dpr));
  canvas.style.width = `${w}px`;
  canvas.style.height = `${h}px`;
  canvas.style.left = `${view.left}px`;
  canvas.style.top = `${view.top}px`;
  const ctx = canvas.getContext('2d');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.translate(canvas.width / 2, canvas.height / 2);
  ctx.rotate((turns * Math.PI) / 2);
  const [dw, dh] = turns % 2 ? [canvas.height, canvas.width] : [canvas.width, canvas.height];
  ctx.imageSmoothingQuality = 'high';
  ctx.drawImage(picture, -dw / 2, -dh / 2, dw, dh);
  place();
}

function place() {
  box.style.left = `${view.left + crop.x * view.scale}px`;
  box.style.top = `${view.top + crop.y * view.scale}px`;
  box.style.width = `${crop.w * view.scale}px`;
  box.style.height = `${crop.h * view.scale}px`;
  // A field being typed in keeps what is typed.
  if (document.activeElement !== widthIn) widthIn.value = String(crop.w);
  if (document.activeElement !== heightIn) heightIn.value = String(crop.h);
}

// Takes the number typed in a size field, or puts the box's back if it isn't one.
function commit(field) {
  if (!session) return;
  const text = field.value.trim();
  const value = Number(text);
  if (/^\d+$/.test(text) && value >= 1) crop = typed(field === widthIn ? 'w' : 'h', value);
  field.value = String(field === widthIn ? crop.w : crop.h);
  place();
}

for (const field of [widthIn, heightIn]) {
  field.addEventListener('change', () => commit(field));
}

function say(text) {
  message.textContent = text;
  message.hidden = false;
}

box.addEventListener('pointerdown', (event) => {
  if (!session || event.button !== 0) return;
  // Takes a number still being typed first, and the arrow keys come to the box.
  box.focus();
  drag = { edge: event.target.dataset.edge ?? 'move', start: { ...crop }, x: event.clientX, y: event.clientY };
  box.setPointerCapture(event.pointerId);
  event.preventDefault();
});

box.addEventListener('pointermove', (event) => {
  if (!drag) return;
  const dx = (event.clientX - drag.x) / view.scale;
  const dy = (event.clientY - drag.y) / view.scale;
  crop = drag.edge === 'move' ? moved(drag.start, dx, dy) : resized(drag.start, drag.edge, dx, dy);
  place();
});

for (const end of ['pointerup', 'pointercancel']) {
  box.addEventListener(end, () => {
    drag = null;
  });
}

for (const button of document.querySelectorAll('[data-ratio]')) {
  button.addEventListener('click', () => {
    const value = button.dataset.ratio;
    shape = value === 'free' ? null : value === 'original' ? 'original' : Number(value);
    for (const other of document.querySelectorAll('[data-ratio]')) {
      other.setAttribute('aria-pressed', String(other === button));
    }
    if (session && shape !== null) crop = fullBox();
    if (session) place();
  });
}

function turn(by) {
  if (!session) return;
  turns = (turns + by + 4) % 4;
  crop = fullBox();
  layout();
}

document.getElementById('left').addEventListener('click', () => turn(-1));
document.getElementById('right').addEventListener('click', () => turn(1));

async function cropIt() {
  if (!session || apply.disabled) return;
  apply.disabled = true;
  try {
    // The app checks the box, queues the crop and closes this window.
    await invoke('editor_apply', { crop: { x: crop.x, y: crop.y, width: crop.w, height: crop.h, turns } });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', cropIt);
document.getElementById('cancel').addEventListener('click', cancel);

// In a size field, Enter takes the number (another Enter then crops), Esc puts the
// box's back, and up and down count one pixel, ten with Shift. Elsewhere Esc cancels,
// Enter crops (unless a button has the focus), the arrow keys move the box a screen
// pixel at a time, ten with Shift.
document.addEventListener('keydown', (event) => {
  const field = [widthIn, heightIn].find((f) => f === document.activeElement);
  if (field) {
    const step = { ArrowUp: 1, ArrowDown: -1 }[event.key];
    if (event.key === 'Enter') {
      event.preventDefault();
      commit(field);
      box.focus();
    } else if (event.key === 'Escape') {
      event.preventDefault();
      field.value = '';
      box.focus();
    } else if (step && session) {
      event.preventDefault();
      const now = Number(field.value) || (field === widthIn ? crop.w : crop.h);
      field.value = String(Math.max(1, now + step * (event.shiftKey ? 10 : 1)));
      commit(field);
      field.select();
    }
    return;
  }
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    cropIt();
  } else if (session && document.activeElement === box) {
    const step = event.shiftKey ? 10 : 1;
    const by = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[event.key];
    if (by) {
      event.preventDefault();
      crop = moved(crop, by[0] / view.scale, by[1] / view.scale);
      place();
    }
  }
});

new ResizeObserver(() => layout()).observe(stage);

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session
    && Number.isInteger(session.width) && session.width > 0
    && Number.isInteger(session.height) && session.height > 0
    && typeof session.preview === 'string' && session.preview.startsWith('data:image/jpeg;base64,');
  if (!valid) {
    session = null;
    apply.disabled = true;
    say('Nothing to crop. Drag a picture onto Crop again.');
    return;
  }
  document.title = `Crop: ${session.name}`;
  picture.onload = () => {
    crop = fullBox();
    box.hidden = false;
    layout();
    box.focus();
  };
  picture.onerror = () => say("The picture couldn't be shown.");
  picture.src = session.preview;
}

load();
