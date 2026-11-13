// The Snapshot window: a slider along the video and the frame at that moment. The app
// hands over only the length and draws each frame asked for; this page sends back
// only the moment, in milliseconds.
if (!window.__TAURI__) throw new Error('Pirouette: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const moment = $('moment');
const frame = $('frame');
const apply = $('apply');
const problem = $('problem');

// { name, length_ms } from the app, once loaded.
let session = null;
// Each frame asked for gets a number; an older answer arriving late is dropped.
let asked = 0;
let timer = 0;

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

// "1:05.2", "0:07.0", "1:02:03.5".
function timeText(ms) {
  const tenths = Math.floor(ms / 100) % 10;
  const total = Math.floor(ms / 1000);
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n) => String(n).padStart(2, '0');
  return (h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`) + `.${tenths}`;
}

function at() {
  return Math.min(session.length_ms, Math.max(0, Math.round(Number(moment.value))));
}

function show() {
  $('hint').textContent = `At ${timeText(at())} of ${timeText(session.length_ms)}. ← → step a frame, with Shift a second.`;
  clearTimeout(timer);
  // A short pause while the slider moves, so ffmpeg isn't asked for every step.
  timer = setTimeout(load, 120);
}

async function load() {
  const mine = ++asked;
  try {
    const url = await invoke('editor_frame', { ms: at() });
    if (mine === asked && typeof url === 'string' && url.startsWith('data:image/jpeg;base64,')) {
      frame.src = url;
      say('');
    }
  } catch (error) {
    if (mine === asked) say(String(error));
  }
}

function step(ms) {
  moment.value = String(Math.min(session.length_ms, Math.max(0, at() + ms)));
  show();
}

async function save() {
  if (!session) return;
  apply.disabled = true;
  try {
    // The app checks the moment, queues the job and closes this window.
    await invoke('editor_snapshot', { ms: at() });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', save);
$('cancel').addEventListener('click', cancel);
moment.addEventListener('input', show);

document.addEventListener('keydown', (event) => {
  if (!session) return;
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    save();
  } else if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
    event.preventDefault();
    const sign = event.key === 'ArrowRight' ? 1 : -1;
    step(sign * (event.shiftKey ? 1000 : 40));
  }
});

async function start() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session && session.kind === 'snapshot' && Number.isSafeInteger(session.length_ms) && session.length_ms > 0;
  if (!valid) {
    session = null;
    apply.disabled = true;
    moment.disabled = true;
    say('Nothing to take a picture of. Drag the video onto Snapshot again.');
    return;
  }
  document.title = `Snapshot: ${session.name}`;
  moment.max = String(session.length_ms);
  moment.value = String(Math.floor(session.length_ms / 2));
  show();
  moment.focus();
}

start();
