// The Info window: what a file says about itself, to read, and for a sound file or a
// PDF its tags, to change. The app reads the file; this page sends back only the tags
// that changed, which the app checks and writes into a copy next to the file
// (src-tauri/src/convert/info.rs).
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const apply = $('apply');
const problem = $('problem');

// { name, what, facts, fields } from the app, once loaded.
let session = null;
// Each tag that can change: its name, its value as read, and its box.
const boxes = [];

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

// The tags whose box no longer holds what was read: [name, new value] each.
function changes() {
  return boxes.filter((b) => b.input.value !== b.value).map((b) => [b.key, b.input.value]);
}

function show() {
  apply.disabled = !session || changes().length === 0;
}

async function save() {
  if (!session || apply.disabled) return;
  apply.disabled = true;
  try {
    // The app checks the tags, queues the job and closes this window.
    await invoke('editor_info', { changes: changes() });
  } catch (error) {
    say(String(error));
    show();
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', save);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    save();
  }
});

function build() {
  const facts = $('facts');
  for (const [label, value] of session.facts) {
    const dt = document.createElement('dt');
    dt.textContent = label;
    const dd = document.createElement('dd');
    dd.textContent = value;
    facts.append(dt, dd);
  }
  const fields = $('fields');
  for (const { key, label, value } of session.fields) {
    const name = document.createElement('label');
    name.htmlFor = `tag-${key}`;
    name.textContent = label;
    const input = document.createElement('input');
    input.id = `tag-${key}`;
    input.value = value;
    input.maxLength = 1000;
    input.autocomplete = 'off';
    input.addEventListener('input', show);
    fields.append(name, input);
    boxes.push({ key, value, input });
  }
  if (boxes.length > 0) {
    $('hint').textContent = 'The new tags go into a copy next to the file.';
    boxes[0].input.focus();
  } else {
    $('hint').textContent = `A ${session.what}'s details can only be read here.`;
    apply.hidden = true;
    $('cancel').textContent = 'Close';
  }
}

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const text = (v) => typeof v === 'string';
  const valid = session
    && session.kind === 'info' && text(session.what)
    && Array.isArray(session.facts) && session.facts.every((f) => Array.isArray(f) && f.length === 2 && f.every(text))
    && Array.isArray(session.fields) && session.fields.every((f) => f && text(f.key) && text(f.label) && text(f.value));
  if (!valid) {
    session = null;
    show();
    say('Nothing to show. Drag the file onto Info again.');
    return;
  }
  document.title = `Info: ${session.name}`;
  $('what').textContent = session.name;
  build();
  show();
}

load();
