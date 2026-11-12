// Float's card on Windows and Linux (float_web.rs). The icon and name take the files
// out; the rest of the card moves it; × puts it away.
if (!window.__TAURI__) throw new Error('Float: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;
const DRAG_START = 4; // px with the button down before it is a drag, not a click

const files = document.getElementById('files');
let card = null;
let press = null;

invoke('float_card').then((c) => {
  if (!c) { invoke('float_close'); return; }
  card = c;
  document.getElementById('title').textContent = c.title;
  document.getElementById('detail').textContent = c.detail;
  // Linux: the page's own drag carries the files as a uri-list.
  files.draggable = !c.native;
});

document.getElementById('close').addEventListener('click', () => invoke('float_close'));

document.getElementById('card').addEventListener('pointerdown', (e) => {
  if (e.button !== 0 || e.target.closest('#close')) return;
  if (e.target.closest('#files')) { press = { x: e.clientX, y: e.clientY }; return; }
  invoke('float_move');
});

// Windows: once the pointer has moved far enough, the system's own file drag.
window.addEventListener('pointermove', (e) => {
  if (!press || !card || !card.native || !(e.buttons & 1)) return;
  if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_START) return;
  press = null;
  invoke('float_drag').catch(() => {});
});
window.addEventListener('pointerup', () => { press = null; });

files.addEventListener('dragstart', (e) => {
  if (!card) { e.preventDefault(); return; }
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData('text/uri-list', card.uris.join('\r\n') + '\r\n');
  e.dataTransfer.setData('text/plain', card.paths.join('\n'));
});
files.addEventListener('dragend', () => invoke('float_dropped'));
