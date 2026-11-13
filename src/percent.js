// The Compress to % window: several files, each to the same share of its own size.
// The app hands over how many and their size together; this page sends back only
// the share, in percent.
if (!window.__TAURI__) throw new Error('Pirouette: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const share = $('share');
const presets = $('presets');
const apply = $('apply');
const problem = $('problem');

// { name, count, bytes, min, max } from the app, once loaded.
let session = null;

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

// "2 MB", "1.5 MB", "500 KB", as Finder counts (1000 bytes to a kilobyte).
function sizeText(bytes) {
  const [value, unit] = bytes >= 1e9 ? [bytes / 1e9, 'GB'] : bytes >= 1e6 ? [bytes / 1e6, 'MB'] : [bytes / 1e3, 'KB'];
  return `${value < 10 ? value.toFixed(1).replace(/\.0$/, '') : Math.round(value)} ${unit}`;
}

function chosen() {
  const text = share.value.trim();
  if (!/^\d{1,3}$/.test(text)) return null;
  const n = Number(text);
  return n >= session.min && n <= session.max ? n : null;
}

function update() {
  for (const button of presets.children) {
    button.setAttribute('aria-pressed', String(button.dataset.share === share.value.trim()));
  }
  const n = chosen();
  apply.disabled = n === null;
  $('hint').textContent = n === null
    ? `Type a number from ${session.min} to ${session.max}.`
    : `About ${sizeText((session.bytes * n) / 100)} together, or less. A file that can't get that small stays as it is, and the notification says so.`;
}

function pick(n) {
  share.value = String(n);
  say('');
  update();
}

async function compress() {
  const n = session && chosen();
  if (!n) return;
  apply.disabled = true;
  try {
    // The app checks the share, queues the files and closes this window.
    await invoke('editor_percent', { percent: n });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', compress);
$('cancel').addEventListener('click', cancel);
share.addEventListener('input', () => {
  say('');
  update();
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    compress();
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
  const valid = session && session.kind === 'percent' && whole(session.count) && whole(session.bytes)
    && whole(session.min) && whole(session.max) && session.min < session.max;
  if (!valid) {
    session = null;
    apply.disabled = true;
    share.disabled = true;
    say('Nothing to compress. Drag the files onto To % again.');
    return;
  }
  document.title = `Compress to %: ${session.name}`;
  $('what').textContent = `${session.count} files, ${sizeText(session.bytes)} together.`;
  for (const n of [75, 50, 25, 10]) {
    if (n < session.min || n > session.max) continue;
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.share = String(n);
    button.textContent = `${n}%`;
    button.addEventListener('click', () => pick(n));
    presets.append(button);
  }
  pick(50);
  share.focus();
  share.select();
}

load();
