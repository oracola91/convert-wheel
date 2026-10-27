// The Compress to a Size window: the file's size, a size to type or pick, and how small
// it can go. The app hands over the file's size and kind; this page sends back only
// the size to aim under, in bytes.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const amount = $('amount');
const unit = $('unit');
const presets = $('presets');
const apply = $('apply');
const problem = $('problem');

// Sizes people aim for (mail attachments, upload forms), largest first.
const COMMON = [25e6, 10e6, 5e6, 2e6, 1e6, 500e3, 200e3, 100e3];
const HINTS = {
  picture: 'The highest quality that fits; if even the lowest is too big, the picture gets smaller too.',
  video: 'The bit rate the size allows over the video’s length, in two passes; a low rate also makes the picture smaller.',
  sound: 'The bit rate the size allows over the sound’s length.',
};

// { name, bytes, what, least } from the app, once loaded.
let session = null;

// "2.8 MB", the way Finder counts (1000 bytes to a kilobyte).
function sizeText(n) {
  if (n < 1000) return `${n} bytes`;
  const [value, name] = n < 1e6 ? [n / 1e3, 'KB'] : n < 1e9 ? [n / 1e6, 'MB'] : [n / 1e9, 'GB'];
  const text = value < 10 ? value.toFixed(1) : value.toFixed(0);
  return `${text.replace(/\.0$/, '')} ${name}`;
}

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

// Shows `bytes` in the box, in MB from a megabyte up, otherwise in KB.
function show(bytes) {
  const mb = bytes >= 1e6;
  unit.value = mb ? '1000000' : '1000';
  const value = bytes / (mb ? 1e6 : 1e3);
  amount.value = String(Number(value.toFixed(value < 10 ? 2 : 1)));
  check();
}

// The size asked for, in bytes, or null when the box isn't a number.
function asked() {
  const text = amount.value.trim().replace(',', '.');
  if (!/^\d+(\.\d+)?$/.test(text)) return null;
  return Math.floor(Number(text) * Number(unit.value));
}

// Says what is wrong with the size asked for, if anything, and whether it can go.
function check() {
  const bytes = asked();
  let text = '';
  if (bytes === null) text = 'Type a size, such as 2 or 1.5.';
  else if (bytes >= session.bytes) text = `That isn't smaller: the file is ${sizeText(session.bytes)}.`;
  else if (bytes < 1000) text = 'At least 1 KB.';
  else if (session.least && bytes < session.least) text = `Too small for its length: at least about ${sizeText(session.least)}.`;
  say(text);
  apply.disabled = text !== '';
  return text === '';
}

function addPreset(label, bytes) {
  const button = document.createElement('button');
  button.type = 'button';
  button.textContent = label;
  button.addEventListener('click', () => show(bytes));
  presets.append(button);
}

async function compress() {
  if (!session || !check()) return;
  apply.disabled = true;
  try {
    // The app checks the size, queues the job and closes this window.
    await invoke('editor_size', { bytes: asked() });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

amount.addEventListener('input', check);
unit.addEventListener('change', check);
apply.addEventListener('click', compress);
$('cancel').addEventListener('click', cancel);

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
  const valid = session
    && session.kind === 'size'
    && Number.isSafeInteger(session.bytes) && session.bytes > 1000
    && Object.hasOwn(HINTS, session.what)
    && (session.least === null || Number.isSafeInteger(session.least));
  if (!valid) {
    session = null;
    apply.disabled = true;
    say('Nothing to compress. Drag the file onto Compress to a Size again.');
    return;
  }
  document.title = `Compress to a size: ${session.name}`;
  $('what').textContent = `${session.name} is ${sizeText(session.bytes)}.`;
  $('hint').textContent = HINTS[session.what];
  const floor = Math.max(1000, session.least ?? 0);
  addPreset('Half', Math.floor(session.bytes / 2));
  addPreset('Quarter', Math.floor(session.bytes / 4));
  for (const bytes of COMMON.filter((b) => b < session.bytes && b >= floor).slice(0, 4)) {
    addPreset(sizeText(bytes), bytes);
  }
  show(Math.max(floor, Math.floor(session.bytes / 2)));
  amount.focus();
  amount.select();
}

load();
