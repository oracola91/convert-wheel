// The Password window: a password for the PDF, typed twice. It goes to the app once,
// when the button is pressed; this page keeps it nowhere and clears the fields then.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const password = $('password');
const again = $('again');
const apply = $('apply');
const problem = $('problem');

// { name, max } from the app, once loaded.
let session = null;

function say(text) {
  problem.textContent = text;
  problem.hidden = !text;
}

// Why the typed password can't be used yet, or '' when it can (the app checks again).
function why() {
  const text = password.value;
  if (!text) return 'Type a password, then the same again.';
  if ([...text].length > session.max) return `At most ${session.max} characters.`;
  if (text.includes(',')) return 'A comma can’t be in it.';
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(text)) return 'Tabs and line breaks can’t be in it.';
  if (again.value !== text) return again.value ? 'The two don’t match yet.' : 'Type it again below.';
  return '';
}

function update() {
  if (!session) return;
  const reason = why();
  apply.disabled = reason !== '';
  $('hint').textContent = reason
    || 'Keep it somewhere safe: without it, the PDF can’t be opened. The original stays as it is.';
}

async function lock() {
  if (!session || why()) return;
  apply.disabled = true;
  try {
    // The app checks it, queues the job and closes this window.
    await invoke('editor_lock', { password: password.value });
    password.value = '';
    again.value = '';
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  password.value = '';
  again.value = '';
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

apply.addEventListener('click', lock);
$('cancel').addEventListener('click', cancel);
for (const field of [password, again]) {
  field.addEventListener('input', () => {
    say('');
    update();
  });
}
$('show').addEventListener('change', (event) => {
  const type = event.target.checked ? 'text' : 'password';
  password.type = type;
  again.type = type;
});

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    if (document.activeElement === password && !again.value) again.focus();
    else lock();
  }
});

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session && session.kind === 'lock' && Number.isSafeInteger(session.max) && session.max > 0;
  if (!valid) {
    session = null;
    apply.disabled = true;
    password.disabled = true;
    again.disabled = true;
    say('Nothing to lock. Drag the PDF onto Password again.');
    return;
  }
  document.title = `Password: ${session.name}`;
  $('what').textContent = `${session.name} will open only with this password.`;
  update();
  password.focus();
}

load();
