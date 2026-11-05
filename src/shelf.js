// The shelf on Windows and Linux (shelf_web.rs). The list takes every held file out at
// once; a click on a name shows that file in the file manager; x lets go of them all.
if (!window.__TAURI__) throw new Error('Shelf: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const SHOWN = 4; // names listed before "and N more"
const DRAG_START = 4;

const root = document.getElementById('shelf');
const list = document.getElementById('list');
const files = document.getElementById('files');
let state = { items: [], native: false, open: false };
let press = null;

function render() {
  root.classList.toggle('pill', !state.open);
  root.classList.toggle('empty', state.items.length === 0);
  const n = state.items.length;
  document.getElementById('count').textContent = n === 1 ? state.items[0].name : `${n} files on the shelf`;
  document.getElementById('title').textContent = n === 0 ? 'Shelf' : n === 1 ? '1 file' : `${n} files`;
  list.replaceChildren();
  for (const item of state.items.slice(0, SHOWN)) {
    const li = document.createElement('li');
    li.textContent = item.name;
    li.title = item.path;
    li.addEventListener('click', () => invoke('shelf_reveal', { path: item.path }));
    list.append(li);
  }
  if (n > SHOWN) {
    const li = document.createElement('li');
    li.className = 'more';
    li.textContent = `and ${n - SHOWN} more`;
    list.append(li);
  }
  // Linux: the page's own drag carries every held file as a uri-list.
  files.draggable = !state.native && n > 0;
}

function refresh() {
  invoke('shelf_items').then((s) => { state = s; render(); }).catch(() => {});
}
listen('shelf-changed', refresh);
refresh();

document.getElementById('pill').addEventListener('mouseenter', () => invoke('shelf_open', { open: true }));
root.addEventListener('mouseleave', () => { if (state.open) invoke('shelf_open', { open: false }); });
document.getElementById('clear').addEventListener('click', () => invoke('shelf_clear'));

// Windows: once the pointer has moved far enough, the system's own file drag.
files.addEventListener('pointerdown', (e) => {
  if (e.button === 0 && state.items.length > 0) press = { x: e.clientX, y: e.clientY };
});
window.addEventListener('pointermove', (e) => {
  if (!press || !state.native || !(e.buttons & 1)) return;
  if (Math.hypot(e.clientX - press.x, e.clientY - press.y) < DRAG_START) return;
  press = null;
  invoke('shelf_drag').catch(() => {});
});
window.addEventListener('pointerup', () => { press = null; });

files.addEventListener('dragstart', (e) => {
  if (state.items.length === 0) { e.preventDefault(); return; }
  e.dataTransfer.effectAllowed = 'copyMove';
  e.dataTransfer.setData('text/uri-list', state.items.map((i) => i.uri).join('\r\n') + '\r\n');
  e.dataTransfer.setData('text/plain', state.items.map((i) => i.path).join('\n'));
});
files.addEventListener('dragend', (e) => invoke('shelf_dragged', { landed: e.dataTransfer.dropEffect !== 'none' }));
