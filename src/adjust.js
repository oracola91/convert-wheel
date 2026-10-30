// The Adjust window: the picture with sliders for brightness, contrast and saturation,
// shown as they go with the browser's own filters. The app hands over the picture;
// this page sends back only the three numbers, which the app checks and applies with
// ImageMagick the same way (adjust_args in src-tauri/src/convert.rs).
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const picture = $('picture');
const apply = $('apply');
const message = $('message');
const SLIDERS = ['brightness', 'contrast', 'saturation'];

// { name, preview } from the app, once loaded.
let session = null;

function say(text) {
  message.textContent = text;
  message.hidden = !text;
}

function values() {
  return Object.fromEntries(SLIDERS.map((id) => [id, Number($(id).value)]));
}

function show() {
  const v = values();
  picture.style.filter = `brightness(${v.brightness / 100}) contrast(${v.contrast / 100}) saturate(${v.saturation / 100})`;
  for (const id of SLIDERS) $(`${id}-value`).textContent = `${v[id]}%`;
  apply.disabled = !session || SLIDERS.every((id) => v[id] === 100);
}

async function adjustIt() {
  if (!session || apply.disabled) return;
  apply.disabled = true;
  try {
    // The app checks the numbers, queues the job and closes this window.
    await invoke('editor_adjust', { adjust: values() });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

for (const id of SLIDERS) $(id).addEventListener('input', show);
$('reset').addEventListener('click', () => {
  for (const id of SLIDERS) $(id).value = '100';
  show();
});
apply.addEventListener('click', adjustIt);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    adjustIt();
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
    && session.kind === 'adjust'
    && typeof session.preview === 'string' && session.preview.startsWith('data:image/jpeg;base64,');
  if (!valid) {
    session = null;
    show();
    say('Nothing to adjust. Drag the picture onto Adjust again.');
    return;
  }
  document.title = `Adjust: ${session.name}`;
  picture.src = session.preview;
  show();
  $('brightness').focus();
}

load();
