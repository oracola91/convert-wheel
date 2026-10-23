// Draws the format wheel. The drop itself is handled natively (drop_view.rs), which
// says what is under the cursor. The glass and the file icon in the hub are native too.
const { listen } = window.__TAURI__.event;

// Same numbers as src-tauri/src/wheel.rs.
const SIZE = 340;
const C = SIZE / 2;
const R_OUT = 160;
const R_IN = 62;
const HUB_R = R_IN - 6;
// How far the segment under the cursor moves out toward it.
const LIFT = 7;
const NS = 'http://www.w3.org/2000/svg';

// What the hub says over a segment. Anything not listed shows in capitals.
const NAMES = {
  jpg: 'JPEG', md: 'Markdown', txt: 'Plain text', html: 'Web page', docx: 'Word',
  odt: 'OpenDocument', xlsx: 'Excel', ods: 'OpenDocument', pptx: 'PowerPoint',
  odp: 'OpenDocument', mov: 'QuickTime', webp: 'WebP', webm: 'WebM', ogg: 'Ogg',
};

const svg = document.getElementById('wheel');
let paths = [];
let formats = [];
let segments = [];
let hub = null;
let hubLabel = null;

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

// Builds the wheel once per drag; hovering only switches classes, so changes animate.
function build() {
  svg.replaceChildren();
  const n = formats.length;
  segments = formats.map((f, i) => {
    const mid = n === 1 ? 0 : ((i + 0.5) / n) * 2 * Math.PI;
    const g = el('g', { class: 'seg' });
    // A single full ring has nowhere to move out to.
    if (n > 1) {
      g.style.setProperty('--dx', `${(LIFT * Math.sin(mid)).toFixed(2)}px`);
      g.style.setProperty('--dy', `${(-LIFT * Math.cos(mid)).toFixed(2)}px`);
    }
    g.append(el('path', { d: sectorPath(i, n), 'fill-rule': 'evenodd' }));
    const [x, y] = polar((R_OUT + R_IN) / 2, mid);
    const label = el('text', { x, y });
    label.textContent = f.toUpperCase();
    g.append(label);
    svg.append(g);
    return g;
  });
  hub = el('circle', { cx: C, cy: C, r: HUB_R, class: 'hub' });
  // The native file icon sits above the centre; the label goes under it.
  hubLabel = el('text', { x: C, y: C + 28, class: 'hub-label' });
  svg.append(hub, hubLabel);
  show({ segment: null, hub: false });
}

function show(hover) {
  segments.forEach((g, i) => g.classList.toggle('active', i === hover.segment));
  hub.classList.toggle('cancel', hover.hub);
  hubLabel.classList.toggle('strong', hover.segment !== null || hover.hub);
  if (hover.hub) {
    hubLabel.textContent = 'Cancel';
  } else if (hover.segment !== null) {
    const f = formats[hover.segment];
    hubLabel.textContent = `To ${NAMES[f] ?? f.toUpperCase()}`;
  } else if (formats.length === 0) {
    hubLabel.textContent = 'No formats';
  } else {
    hubLabel.textContent = paths.length === 1 ? '1 file' : `${paths.length} files`;
  }
}

function restart(className) {
  svg.classList.remove('opening', 'closing');
  void svg.getBoundingClientRect();
  if (className) svg.classList.add(className);
}

listen('wheel-show', (event) => {
  paths = event.payload.paths;
  formats = event.payload.formats;
  build();
  restart('opening');
});

listen('wheel-hover', (event) => show(event.payload));

// The chosen segment stays lit while the wheel shrinks back to where it opened.
listen('wheel-close', () => restart('closing'));
