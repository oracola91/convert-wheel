// The Speed window: how long the file plays, a speed to pick, and how long it will play
// then. The app hands over the length and the speeds it takes; this page sends back
// only the speed chosen, in percent.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const speeds = $('speeds');
const apply = $('apply');
const problem = $('problem');

// { name, length_ms, what, speeds } from the app, once loaded.
let session = null;
let chosen = 200;

// "1:05", "0:07", "1:02:03".
function timeText(ms) {
  const total = Math.round(ms / 1000);
  const [h, m, s] = [Math.floor(total / 3600), Math.floor(total / 60) % 60, total % 60];
  const pad = (n) => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// "0.5×" for 50, "2×" for 200.
const times = (percent) => `${percent / 100}×`;

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

function choose(percent) {
  chosen = percent;
  for (const button of speeds.children) {
    button.setAttribute('aria-pressed', String(Number(button.dataset.percent) === percent));
  }
  const after = timeText((session.length_ms * 100) / percent);
  const pitch = session.what === 'video' ? 'The sound keeps its pitch.' : 'It keeps its pitch.';
  $('hint').textContent = `At ${times(percent)} it plays for ${after}. ${pitch}`;
}

async function change() {
  if (!session) return;
  apply.disabled = true;
  try {
    // The app checks the speed, queues the job and closes this window.
    await invoke('editor_speed', { percent: chosen });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', change);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    change();
  } else if (session && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
    // Left is slower, right is faster, as the buttons are laid out.
    const list = session.speeds;
    const at = list.indexOf(chosen) + (event.key === 'ArrowRight' ? 1 : -1);
    if (at >= 0 && at < list.length) choose(list[at]);
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
    && session.kind === 'speed'
    && Number.isSafeInteger(session.length_ms) && session.length_ms > 0
    && ['video', 'sound'].includes(session.what)
    && Array.isArray(session.speeds) && session.speeds.length > 0
    && session.speeds.every((p) => Number.isSafeInteger(p) && p > 0);
  if (!valid) {
    session = null;
    apply.disabled = true;
    say('Nothing to change. Drag the file onto Speed again.');
    return;
  }
  document.title = `Speed: ${session.name}`;
  $('what').textContent = `${session.name} plays for ${timeText(session.length_ms)}.`;
  for (const percent of session.speeds) {
    const button = document.createElement('button');
    button.type = 'button';
    button.dataset.percent = String(percent);
    button.textContent = times(percent);
    button.addEventListener('click', () => choose(percent));
    speeds.append(button);
  }
  choose(session.speeds.includes(200) ? 200 : session.speeds[0]);
  apply.focus();
}

load();
