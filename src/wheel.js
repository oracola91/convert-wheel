// Draws the format wheel. The drop itself is handled natively (drop_view.rs), which
// also says which segment is under the cursor.
const { listen } = window.__TAURI__.event;

// Same numbers as src-tauri/src/wheel.rs.
const SIZE = 340;
const C = SIZE / 2;
const R_OUT = 160;
const R_IN = 62;
const NS = 'http://www.w3.org/2000/svg';

const svg = document.getElementById('wheel');
let paths = [];
let formats = [];
let active = -1;

function el(name, attrs) {
  const node = document.createElementNS(NS, name);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, v);
  return node;
}

// Angle 0 points up and grows clockwise, like a clock face.
function polar(r, a) {
  return [C + r * Math.sin(a), C - r * Math.cos(a)];
}

function sectorPath(i, n) {
  if (n === 1) {
    // A full ring: two half circles each way, the hole cut out by the even-odd rule.
    return `M ${C} ${C - R_OUT} A ${R_OUT} ${R_OUT} 0 1 1 ${C} ${C + R_OUT} A ${R_OUT} ${R_OUT} 0 1 1 ${C} ${C - R_OUT} Z ` +
      `M ${C} ${C - R_IN} A ${R_IN} ${R_IN} 0 1 0 ${C} ${C + R_IN} A ${R_IN} ${R_IN} 0 1 0 ${C} ${C - R_IN} Z`;
  }
  const gap = 0.025;
  const a0 = (i / n) * 2 * Math.PI + gap;
  const a1 = ((i + 1) / n) * 2 * Math.PI - gap;
  const large = a1 - a0 > Math.PI ? 1 : 0;
  const [x0, y0] = polar(R_OUT, a0);
  const [x1, y1] = polar(R_OUT, a1);
  const [x2, y2] = polar(R_IN, a1);
  const [x3, y3] = polar(R_IN, a0);
  return `M ${x0} ${y0} A ${R_OUT} ${R_OUT} 0 ${large} 1 ${x1} ${y1} L ${x2} ${y2} A ${R_IN} ${R_IN} 0 ${large} 0 ${x3} ${y3} Z`;
}

function hubText() {
  if (formats.length === 0) return 'No formats';
  if (active >= 0) return `To ${formats[active].toUpperCase()}`;
  return paths.length === 1 ? '1 file' : `${paths.length} files`;
}

function draw() {
  svg.replaceChildren();
  const n = formats.length;
  formats.forEach((f, i) => {
    const g = el('g', { class: i === active ? 'seg active' : 'seg' });
    g.append(el('path', { d: sectorPath(i, n), 'fill-rule': 'evenodd' }));
    const [x, y] = polar((R_OUT + R_IN) / 2, n === 1 ? 0 : ((i + 0.5) / n) * 2 * Math.PI);
    const label = el('text', { x, y });
    label.textContent = f.toUpperCase();
    g.append(label);
    svg.append(g);
  });
  svg.append(el('circle', { cx: C, cy: C, r: R_IN - 6, class: 'hub' }));
  const label = el('text', { x: C, y: C, class: 'hub-label' });
  label.textContent = hubText();
  svg.append(label);
}

function setActive(i) {
  if (i === active) return;
  active = i;
  draw();
}

listen('wheel-show', (event) => {
  paths = event.payload.paths;
  formats = event.payload.formats;
  active = -1;
  draw();
  svg.classList.remove('pop');
  void svg.getBoundingClientRect();
  svg.classList.add('pop');
});

listen('wheel-hover', (event) => {
  setActive(event.payload ?? -1);
});

draw();
