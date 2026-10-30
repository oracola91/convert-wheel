// The Split window for sound and video: how long the file plays, how many equal parts
// or how long each part, and what that makes. The app hands over the length; this page
// sends back only the length of a part, in milliseconds, which the app checks again.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const apply = $('apply');
const problem = $('problem');

// Same rules as parts_count in src-tauri/src/convert.rs.
const MAX_PARTS = 50;
const COUNTS = [2, 3, 4, 5, 6, 8, 10];
const LENGTHS = [10e3, 30e3, 60e3, 120e3, 300e3, 600e3, 900e3, 1800e3];

// { name, length_ms, what } from the app, once loaded.
let session = null;
let chosen = null;

// "1:05", "0:07", "1:02:03".
function timeText(ms) {
  const total = Math.round(ms / 1000);
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// "30 s", "5 min".
const lengthText = (ms) => (ms < 60e3 ? `${ms / 1e3} s` : `${ms / 60e3} min`);

// How many parts of `every` the file makes, or null when that isn't from two to
// MAX_PARTS parts of a second or more. A short end goes onto the last part.
function partsCount(every) {
  const length = session.length_ms;
  if (every < 1000) return null;
  const count = Math.floor(length / every) + (length % every >= 1000 ? 1 : 0);
  return count >= 2 && count <= MAX_PARTS ? count : null;
}

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

function choose(every) {
  chosen = every;
  for (const button of document.querySelectorAll('.group button')) {
    button.setAttribute('aria-pressed', String(Number(button.dataset.every) === every));
  }
  const count = partsCount(every);
  const last = session.length_ms - (count - 1) * every;
  const each = timeText(every);
  $('hint').textContent = Math.abs(last - every) < 1000
    ? `${count} parts of ${each} each, in a new folder next to the file.`
    : `${count} parts of ${each}, the last ${timeText(last)}, in a new folder next to the file.`;
  apply.disabled = false;
}

function addChoice(group, label, every) {
  if (partsCount(every) === null) return;
  const button = document.createElement('button');
  button.type = 'button';
  button.dataset.every = String(every);
  button.textContent = label;
  button.addEventListener('click', () => choose(every));
  group.append(button);
}

async function split() {
  if (!session || chosen === null) return;
  apply.disabled = true;
  try {
    // The app checks the length, queues the job and closes this window.
    await invoke('editor_parts', { everyMs: chosen });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', split);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    split();
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
    && session.kind === 'parts'
    && Number.isSafeInteger(session.length_ms) && session.length_ms > 0
    && ['video', 'sound'].includes(session.what);
  apply.disabled = true;
  if (!valid) {
    session = null;
    say('Nothing to split. Drag the file onto Split again.');
    return;
  }
  document.title = `Split: ${session.name}`;
  $('what').textContent = `${session.name} plays for ${timeText(session.length_ms)}.`;
  for (const count of COUNTS) {
    // Only where it really makes that many: rounding can leave one short.
    const every = Math.ceil(session.length_ms / count);
    if (partsCount(every) === count) addChoice($('counts'), String(count), every);
  }
  for (const every of LENGTHS) addChoice($('lengths'), lengthText(every), every);
  for (const id of ['counts', 'lengths']) {
    const empty = $(id).children.length === 0;
    $(id).hidden = empty;
    $(`${id}-label`).hidden = empty;
  }
  const first = document.querySelector('.group button');
  if (!first) {
    say('This is too short to split: each part has to be a second or more.');
    return;
  }
  choose(Number(first.dataset.every));
  apply.focus();
}

load();
