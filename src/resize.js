// The Resize window: the picture's or video's size, a longest side to type or pick,
// and the size it will have. The app hands over the size and the limits; this page
// sends back only the longest side chosen, in pixels.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const side = $('side');
const presets = $('presets');
const apply = $('apply');
const problem = $('problem');

// { name, width, height, what, min, max } from the app, once loaded.
let session = null;

// Common longest sides; a video's named by the height they give in landscape.
const PICTURE_SIDES = [3840, 2560, 1920, 1600, 1280, 1024, 800, 640];
const VIDEO_SIDES = [[3840, '4K'], [2560, '1440p'], [1920, '1080p'], [1280, '720p'], [854, '480p'], [640, '360p']];

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

const longest = () => Math.max(session.width, session.height);

// The size `n` gives, the other side in proportion (even for a video, as encoders need).
function sizeFor(n) {
  const scale = n / longest();
  let [w, h] = [Math.round(session.width * scale), Math.round(session.height * scale)];
  if (session.what === 'video') {
    if (session.width >= session.height) h = Math.max(2, Math.round(h / 2) * 2);
    else w = Math.max(2, Math.round(w / 2) * 2);
  }
  return [w, h];
}

// The typed number, or null when it isn't one the app takes.
function chosen() {
  const text = side.value.trim();
  if (!/^\d{1,6}$/.test(text)) return null;
  const n = Number(text);
  return n >= session.min && n <= session.max && n !== longest() ? n : null;
}

function update() {
  for (const button of presets.children) {
    button.setAttribute('aria-pressed', String(button.dataset.side === side.value.trim()));
  }
  const n = chosen();
  apply.disabled = n === null;
  if (n === null) {
    const typed = side.value.trim();
    $('hint').textContent = typed && Number(typed) === longest()
      ? 'That is the size it has already.'
      : `Type a number from ${session.min} to ${session.max}.`;
    return;
  }
  const [w, h] = sizeFor(n);
  const way = n > longest() ? 'larger' : 'smaller';
  $('hint').textContent = `It will be ${w} × ${h} pixels (${way}).`;
}

function pick(n) {
  side.value = String(n);
  say('');
  update();
}

async function resize() {
  const n = session && chosen();
  if (!n) return;
  apply.disabled = true;
  try {
    // The app checks the size, queues the job and closes this window.
    await invoke('editor_resize', { side: n });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', resize);
$('cancel').addEventListener('click', cancel);
side.addEventListener('input', () => {
  say('');
  update();
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    resize();
  }
});

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const whole = (n) => Number.isSafeInteger(n) && n > 0;
  const valid = session
    && session.kind === 'resize'
    && whole(session.width) && whole(session.height)
    && ['picture', 'video'].includes(session.what)
    && whole(session.min) && whole(session.max) && session.min < session.max;
  if (!valid) {
    session = null;
    apply.disabled = true;
    side.disabled = true;
    say('Nothing to resize. Drag the file onto Resize again.');
    return;
  }
  document.title = `Resize: ${session.name}`;
  $('what').textContent = `${session.name} is ${session.width} × ${session.height} pixels.`;
  const sides = session.what === 'video' ? VIDEO_SIDES : PICTURE_SIDES.map((n) => [n, String(n)]);
  for (const [n, label] of sides) {
    if (n === longest() || n < session.min || n > session.max) continue;
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.side = String(n);
    button.textContent = label;
    button.title = `${n} pixels on the longest side`;
    button.addEventListener('click', () => pick(n));
    presets.append(button);
  }
  // A start that makes it smaller: the first common size under its own, or half.
  const under = sides.map(([n]) => n).find((n) => n < longest() && n >= session.min);
  pick(under ?? Math.max(session.min, Math.round(longest() / 2)));
  side.focus();
  side.select();
}

load();
