// Draws the wheel: formats, (Option/Alt+Shift) tools or (Control+Shift) actions.
// The drop itself is handled natively (drop_view.rs), which
// says what is under the cursor. The glass and the file icon in the hub are native too.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { listen } = window.__TAURI__.event;
const { invoke } = window.__TAURI__.core;

// Same numbers as src-tauri/src/wheel.rs.
const SIZE = 340;
const C = SIZE / 2;
const R_OUT = 160;
const R_IN = 62;
const HUB_R = R_IN - 6;
// How far the segment under the cursor moves out toward it.
const LIFT = 7;
// Rounded corners, so each segment reads as a soft bubble.
const CORNER = 12;
const NS = 'http://www.w3.org/2000/svg';

// What the hub says over a segment. Anything not listed shows in capitals.
const NAMES = {
  jpg: 'JPEG', md: 'Markdown', txt: 'Plain text', html: 'Web page', docx: 'Word',
  xlsx: 'Excel', pptx: 'PowerPoint', mov: 'QuickTime', webp: 'WebP', webm: 'WebM', ogg: 'Ogg',
  ico: 'Icon', opus: 'Opus', wma: 'Windows Media', wmv: 'Windows Media', vtt: 'WebVTT',
};

// The Tools and Actions wheels: what each segment says, and what the hub says over it.
const TOOLS = {
  'tool:compress': ['Compress', 'Compress'],
  'tool:half': ['Half size', 'Half size'],
  'tool:clean': ['No metadata', 'Remove metadata'],
  'tool:rotate': ['Rotate', 'Rotate right'],
  'tool:onepdf': ['One PDF', 'Into one PDF'],
  'tool:merge': ['Merge', 'Merge into one'],
  'tool:split': ['Split', 'One file per page'],
  'tool:smaller': ['Smaller', 'Make smaller'],
  'tool:still': ['First frame', 'First frame as JPEG'],
  'tool:loudness': ['Loudness', 'Even out loudness'],
  'tool:mono': ['Mono', 'Mix down to mono'],
  'act:path': ['Copy path', 'Copy path'],
  'act:show': ['Show', 'Show in folder'],
  'act:terminal': ['Terminal', 'Open Terminal here'],
  'act:airdrop': ['AirDrop', 'Send with AirDrop'],
  'act:zip': ['ZIP', 'Make a ZIP'],
  'act:tgz': ['TAR.GZ', 'Make a TAR.GZ'],
  'act:tar': ['TAR', 'Make a TAR'],
  'act:unpack': ['Unpack', 'Unpack here'],
};

const svg = document.getElementById('wheel');
let paths = [];
// Formats, or tool ids ("tool:half") or action ids ("act:path").
let formats = [];
// "formats", "tools" or "actions".
let wheel = 'formats';
// Windows and Linux learn which files are dragged only when they reach the wheel.
let pending = false;
// Opened from the keyboard shortcut (Linux): waits for a file instead of a mouse-up.
let pinned = false;
let segments = [];
let hub = null;
let hubLabel = null;
// Between wheel-show and wheel-close. Late events for a closed wheel are ignored.
let open = false;
// Counts closes, so clearing the page after one doesn't hit a wheel opened since.
let closes = 0;
let lastHover = { segment: null, hub: false };

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
  const gap = 0.03;
  const a0 = (i / n) * 2 * Math.PI + gap;
  const a1 = ((i + 1) / n) * 2 * Math.PI - gap;
  // Each corner is a small circle touching the edge and the ring; a narrow segment
  // gets smaller corners, so the two at the hub never meet.
  let rc = CORNER;
  while (rc > 2 && 2 * Math.asin(rc / (R_IN + rc)) > 0.8 * (a1 - a0)) rc -= 1;
  const outer = Math.asin(rc / (R_OUT - rc));
  const inner = Math.asin(rc / (R_IN + rc));
  const edgeOut = Math.sqrt((R_OUT - rc) ** 2 - rc ** 2);
  const edgeIn = Math.sqrt((R_IN + rc) ** 2 - rc ** 2);
  const pt = (r, a) => polar(r, a).map((v) => v.toFixed(2)).join(' ');
  const large = (span) => (span > Math.PI ? 1 : 0);
  return [
    `M ${pt(edgeOut, a0)}`,
    `A ${rc} ${rc} 0 0 1 ${pt(R_OUT, a0 + outer)}`,
    `A ${R_OUT} ${R_OUT} 0 ${large(a1 - a0 - 2 * outer)} 1 ${pt(R_OUT, a1 - outer)}`,
    `A ${rc} ${rc} 0 0 1 ${pt(edgeOut, a1)}`,
    `L ${pt(edgeIn, a1)}`,
    `A ${rc} ${rc} 0 0 1 ${pt(R_IN, a1 - inner)}`,
    `A ${R_IN} ${R_IN} 0 ${large(a1 - a0 - 2 * inner)} 0 ${pt(R_IN, a0 + inner)}`,
    `A ${rc} ${rc} 0 0 1 ${pt(edgeIn, a0)}`,
    'Z',
  ].join(' ');
}

// The shine every segment and the hub get, as on a bubble lit from above: bright at
// the top fading out, and a rim that catches the light.
function sheen() {
  const defs = el('defs', {});
  const gradient = (id, stops) => {
    const g = el('linearGradient', { id, x1: 0, y1: 0, x2: 0, y2: 1 });
    for (const [offset, cls] of stops) g.append(el('stop', { offset, class: cls }));
    return g;
  };
  defs.append(
    gradient('sheen', [[0, 'sheen-top'], [0.5, 'sheen-mid'], [1, 'sheen-end']]),
    gradient('rim', [[0, 'rim-top'], [0.55, 'rim-mid'], [1, 'rim-end']]),
  );
  return defs;
}

// Builds the wheel once per drag; hovering only switches classes, so changes animate.
function build() {
  svg.replaceChildren(sheen());
  // Without the system's glass behind it (Windows), the page draws its own disc.
  svg.append(el('circle', { cx: C, cy: C, r: R_OUT, class: 'disc' }));
  const n = formats.length;
  segments = formats.map((f, i) => {
    const mid = n === 1 ? 0 : ((i + 0.5) / n) * 2 * Math.PI;
    const g = el('g', { class: 'seg' });
    // A single full ring has nowhere to move out to.
    if (n > 1) {
      g.style.setProperty('--dx', `${(LIFT * Math.sin(mid)).toFixed(2)}px`);
      g.style.setProperty('--dy', `${(-LIFT * Math.cos(mid)).toFixed(2)}px`);
    }
    const d = sectorPath(i, n);
    g.append(el('path', { d, 'fill-rule': 'evenodd', class: 'body' }), el('path', { d, 'fill-rule': 'evenodd', class: 'shine' }));
    const [x, y] = polar((R_OUT + R_IN) / 2, mid);
    const label = el('text', { x, y });
    label.textContent = Object.hasOwn(TOOLS, f) ? TOOLS[f][0] : f.toUpperCase();
    g.append(label);
    svg.append(g);
    return g;
  });
  hub = el('circle', { cx: C, cy: C, r: HUB_R, class: 'hub' });
  const hubShine = el('circle', { cx: C, cy: C, r: HUB_R, class: 'shine' });
  // The native file icon sits above the centre; the label goes under it.
  hubLabel = el('text', { x: C, y: C + 28, class: 'hub-label' });
  svg.append(hub, hubShine, hubLabel);
  show({ segment: null, hub: false });
}

// Hover messages come from native code; anything unexpected counts as no hover.
function clean(hover) {
  const seg = Number.isInteger(hover?.segment) && hover.segment < formats.length ? hover.segment : null;
  return { segment: seg, hub: hover?.hub === true };
}

function show(raw) {
  if (!hub) return;
  const hover = clean(raw);
  lastHover = hover;
  segments.forEach((g, i) => g.classList.toggle('active', i === hover.segment));
  hub.classList.toggle('cancel', hover.hub);
  hubLabel.classList.toggle('strong', hover.segment !== null || hover.hub);
  if (hover.hub) {
    hubLabel.textContent = 'Cancel';
  } else if (hover.segment !== null) {
    const f = formats[hover.segment];
    if (Object.hasOwn(TOOLS, f)) hubLabel.textContent = TOOLS[f][1];
    else hubLabel.textContent = `To ${Object.hasOwn(NAMES, f) ? NAMES[f] : f.toUpperCase()}`;
  } else if (pending) {
    hubLabel.textContent = pinned ? 'Drag a file here' : 'Drop here';
  } else if (formats.length === 0) {
    hubLabel.textContent = { tools: 'No tools', actions: 'No actions' }[wheel] ?? 'No formats';
  } else if (pinned) {
    hubLabel.textContent = 'Esc to close';
  } else {
    hubLabel.textContent = paths.length === 1 ? '1 file' : `${paths.length} files`;
  }
}

function restart(className) {
  svg.classList.remove('opening', 'closing');
  void svg.getBoundingClientRect();
  if (className) svg.classList.add(className);
}

// The payload comes from the app itself, but a wheel half drawn from bad data would
// be stuck on screen; anything unexpected becomes an empty list.
function readWheel(payload) {
  const list = (value) => (Array.isArray(value) ? value.filter((v) => typeof v === 'string') : []);
  return {
    paths: list(payload?.paths),
    formats: list(payload?.formats),
    pending: payload?.pending === true,
    wheel: ['tools', 'actions'].includes(payload?.wheel) ? payload.wheel : 'formats',
  };
}

listen('wheel-show', (event) => {
  ({ paths, formats, pending, wheel } = readWheel(event.payload));
  const names = { tools: 'Tools for the file', actions: 'Actions for the file' };
  svg.setAttribute('aria-label', names[wheel] ?? 'Formats to convert to');
  pinned = event.payload?.pinned === true;
  document.documentElement.classList.toggle('solid', event.payload?.glass !== true);
  build();
  open = true;
  restart('opening');
});

// The files arrived (Windows, Linux): draw their formats without replaying the open,
// and keep lit whatever the cursor is already over.
listen('wheel-formats', (event) => {
  if (!open) return;
  const keep = lastHover;
  ({ paths, formats, pending, wheel } = readWheel(event.payload));
  build();
  show(keep);
});

listen('wheel-hover', (event) => {
  if (open) show(event.payload);
});

// A chosen segment stays lit while the wheel shrinks back to where it opened. If
// nothing was chosen, nothing stays lit.
listen('wheel-close', (event) => {
  open = false;
  if (event.payload !== true) show({ segment: null, hub: false });
  restart('closing');
  // Once the window is gone, empty the page, so the next wheel never shows this one
  // for a frame before it is drawn.
  const thisClose = ++closes;
  setTimeout(() => {
    if (!open && closes === thisClose) {
      svg.replaceChildren();
      segments = [];
      hub = null;
      hubLabel = null;
    }
  }, 250);
});

// The native code takes every drop. Should one ever reach the page, it must not make
// the webview open the file.
for (const type of ['dragover', 'drop']) {
  document.addEventListener(type, (event) => event.preventDefault());
}

// A wheel opened from the shortcut closes with Esc or a click in the middle.
function cancel() {
  invoke('wheel_cancel').catch((e) => console.error('wheel_cancel failed', e));
}
document.addEventListener('keydown', (event) => {
  if (open && pinned && event.key === 'Escape') cancel();
});
svg.addEventListener('click', (event) => {
  if (open && pinned && event.target === hub) cancel();
});
