// The Trim window: a sound's loudness along its length (and a video's frames above
// it), a part to keep with a start and an end to drag, the times to type, and the
// part to play before trimming. The app hands over the length, the loudness, the
// frames and a small MP3; this page sends back only the two times, in milliseconds.
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const timeline = $('timeline');
const wave = $('wave');
const strip = $('strip');
const keep = $('keep');
const playhead = $('playhead');
const startIn = $('start');
const endIn = $('end');
const lengthOut = $('length');
const play = $('play');
const apply = $('apply');
const sound = $('sound');
const message = $('message');

// The shortest part to keep, as the app requires.
const MIN_MS = 100;

// { name, length_ms, peaks, strip, sound } from the app, once loaded.
let session = null;
let start = 0;
let end = 0;
// The drag under way: 'start', 'end', or 'move' with where it began.
let drag = null;
let playing = false;

function clamp(v, lo, hi) {
  return Math.min(Math.max(v, lo), hi);
}

// "1:23.4", or "1:02:03.4" from an hour on.
function fmt(ms) {
  const tenths = Math.round(ms / 100);
  let seconds = Math.floor(tenths / 10);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  seconds %= 60;
  const mm = hours ? `${hours}:${String(minutes).padStart(2, '0')}` : String(minutes);
  return `${mm}:${String(seconds).padStart(2, '0')}.${tenths % 10}`;
}

// What fmt writes, or plain seconds ("83.4"): milliseconds, or null.
function parse(text) {
  const parts = text.trim().split(':');
  if (parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p.trim()))) return null;
  return Math.round(parts.reduce((total, p) => total * 60 + Number(p), 0) * 1000);
}

function msAt(clientX) {
  const r = timeline.getBoundingClientRect();
  return clamp(Math.round(((clientX - r.left) / r.width) * session.length_ms), 0, session.length_ms);
}

function say(text) {
  message.textContent = text;
  message.hidden = false;
}

// The loudness as bars, sharp on any screen.
function drawWave() {
  if (!session) return;
  const r = wave.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  wave.width = Math.max(1, Math.round(r.width * dpr));
  wave.height = Math.max(1, Math.round(r.height * dpr));
  const ctx = wave.getContext('2d');
  ctx.clearRect(0, 0, wave.width, wave.height);
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--wave').trim();
  const peaks = session.peaks;
  const bar = wave.width / peaks.length;
  const mid = wave.height / 2;
  for (let i = 0; i < peaks.length; i += 1) {
    const h = Math.max(dpr, (peaks[i] / 255) * wave.height * 0.9);
    ctx.fillRect(i * bar, mid - h / 2, Math.max(dpr, bar - 0.5 * dpr), h);
  }
}

function place() {
  const length = session.length_ms;
  keep.style.left = `${(start / length) * 100}%`;
  keep.style.width = `${((end - start) / length) * 100}%`;
  if (document.activeElement !== startIn) startIn.value = fmt(start);
  if (document.activeElement !== endIn) endIn.value = fmt(end);
  lengthOut.textContent = `Keeps ${fmt(end - start)} of ${fmt(length)}`;
  for (const grip of keep.querySelectorAll('.grip')) {
    const at = grip.dataset.end === 'start' ? start : end;
    grip.setAttribute('aria-valuemin', '0');
    grip.setAttribute('aria-valuemax', String(length));
    grip.setAttribute('aria-valuenow', String(at));
    grip.setAttribute('aria-valuetext', fmt(at));
  }
}

function setEnd(which, ms) {
  if (which === 'start') start = clamp(ms, 0, end - MIN_MS);
  else end = clamp(ms, start + MIN_MS, session.length_ms);
  place();
}

function stop() {
  if (!playing) return;
  playing = false;
  sound.pause();
  play.textContent = '▶ Play';
  playhead.hidden = true;
}

function tick() {
  if (!playing) return;
  const ms = sound.currentTime * 1000;
  if (ms >= end || sound.ended) {
    stop();
    return;
  }
  playhead.style.left = `${(ms / session.length_ms) * 100}%`;
  requestAnimationFrame(tick);
}

// Plays the part to keep, from its start; again to stop.
async function toggle() {
  if (!session || !session.sound) return;
  if (playing) {
    stop();
    return;
  }
  sound.currentTime = start / 1000;
  try {
    await sound.play();
  } catch (error) {
    say("The sound couldn't be played.");
    return;
  }
  playing = true;
  play.textContent = '■ Stop';
  playhead.hidden = false;
  tick();
}

timeline.addEventListener('pointerdown', (event) => {
  if (!session || event.button !== 0) return;
  stop();
  const at = msAt(event.clientX);
  const grip = event.target.dataset.end;
  if (grip) {
    drag = { which: grip };
  } else if (event.target === keep) {
    drag = { which: 'move', from: at, start, end };
  } else {
    // Outside the part: the nearer end comes to the pointer.
    drag = { which: Math.abs(at - start) < Math.abs(at - end) ? 'start' : 'end' };
    setEnd(drag.which, at);
  }
  timeline.setPointerCapture(event.pointerId);
  event.preventDefault();
});

timeline.addEventListener('pointermove', (event) => {
  if (!drag) return;
  const at = msAt(event.clientX);
  if (drag.which === 'move') {
    const span = drag.end - drag.start;
    start = clamp(drag.start + at - drag.from, 0, session.length_ms - span);
    end = start + span;
    place();
  } else {
    setEnd(drag.which, at);
  }
});

for (const done of ['pointerup', 'pointercancel']) {
  timeline.addEventListener(done, () => {
    drag = null;
  });
}

for (const [input, which] of [[startIn, 'start'], [endIn, 'end']]) {
  input.addEventListener('change', () => {
    const ms = parse(input.value);
    if (ms !== null) setEnd(which, ms);
    input.value = fmt(which === 'start' ? start : end);
  });
}

async function trimIt() {
  if (!session || apply.disabled) return;
  stop();
  apply.disabled = true;
  try {
    // The app checks the times, queues the trim and closes this window.
    await invoke('editor_trim', { trim: { start_ms: start, end_ms: end } });
  } catch (error) {
    say(String(error));
    apply.disabled = false;
  }
}

function cancel() {
  stop();
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

play.addEventListener('click', toggle);
apply.addEventListener('click', trimIt);
$('cancel').addEventListener('click', cancel);

// Esc cancels, Enter trims and Space plays (outside the time boxes and buttons); on a
// focused end the arrow keys move it a tenth of a second, a second with Shift.
document.addEventListener('keydown', (event) => {
  const typing = document.activeElement instanceof HTMLInputElement;
  const onButton = document.activeElement instanceof HTMLButtonElement;
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !onButton) {
    if (typing) document.activeElement.blur();
    event.preventDefault();
    trimIt();
  } else if (event.key === ' ' && !typing && !onButton) {
    event.preventDefault();
    toggle();
  } else if (session && document.activeElement?.classList.contains('grip')) {
    const step = event.shiftKey ? 1000 : 100;
    const by = { ArrowLeft: -step, ArrowRight: step }[event.key];
    if (by) {
      event.preventDefault();
      const which = document.activeElement.dataset.end;
      setEnd(which, (which === 'start' ? start : end) + by);
    }
  }
});

new ResizeObserver(() => drawWave()).observe(timeline);

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session
    && session.kind === 'trim'
    && Number.isInteger(session.length_ms) && session.length_ms > MIN_MS
    && Array.isArray(session.peaks) && session.peaks.length > 0
    && (session.strip === null || (typeof session.strip === 'string' && session.strip.startsWith('data:image/jpeg;base64,')))
    && (session.sound === null || (typeof session.sound === 'string' && session.sound.startsWith('data:audio/mpeg;base64,')));
  if (!valid) {
    session = null;
    apply.disabled = true;
    say('Nothing to trim. Drag the file onto Trim again.');
    return;
  }
  document.title = `Trim: ${session.name}`;
  end = session.length_ms;
  if (session.strip) {
    strip.src = session.strip;
    strip.hidden = false;
    timeline.classList.add('video');
  }
  if (session.sound) {
    sound.src = session.sound;
    play.disabled = false;
  } else {
    play.title = 'Too long to play here';
  }
  drawWave();
  place();
  keep.querySelector('.grip').focus();
}

load();
