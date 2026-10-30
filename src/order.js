// The Reorder window: a small picture of every page of a PDF, to drag into a new
// order. A page picked with a click or the arrow keys also moves with Shift and the
// arrow keys, or with the buttons. The app hands over the pictures; this page sends
// back only the page numbers in their new order, which the app checks and hands to
// mutool (pdf_tool in src-tauri/src/convert.rs).
if (!window.__TAURI__) throw new Error('Convert Wheel: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const list = $('pages');
const apply = $('apply');
const message = $('message');
// How far the pointer moves before a press becomes a drag, and how near the top or
// bottom of the stage the list starts to scroll while dragging.
const DRAG_START = 5;
const EDGE = 48;

// { name, pages } from the app, once loaded.
let session = null;
// The page the keys and the buttons move (an <li>), once there are pages.
let picked = null;
// While a page is dragged: the page, the pointer (where it went down, where it is,
// where on the page it holds it) and, once it has moved far enough, the copy that
// follows it.
let drag = null;

function say(text) {
  message.textContent = text;
  message.hidden = !text;
}

function tiles() {
  return [...list.children];
}

// The page numbers, from 1, in the order shown.
function order() {
  return tiles().map((li) => Number(li.dataset.page));
}

function show() {
  const all = tiles();
  all.forEach((li, i) => {
    li.setAttribute('aria-label', `Page ${li.dataset.page}, now ${i + 1} of ${all.length}`);
    li.classList.toggle('moved', Number(li.dataset.page) !== i + 1);
  });
  const at = picked ? all.indexOf(picked) : -1;
  $('earlier').disabled = at <= 0;
  $('later').disabled = at < 0 || at === all.length - 1;
  $('reverse').disabled = !session;
  const changed = order().some((page, i) => page !== i + 1);
  $('reset').disabled = !changed;
  apply.disabled = !session || !changed;
}

function pick(li, focus = true) {
  if (picked) {
    picked.setAttribute('aria-selected', 'false');
    picked.tabIndex = -1;
  }
  picked = li;
  li.setAttribute('aria-selected', 'true');
  li.tabIndex = 0;
  if (focus) li.focus();
  li.scrollIntoView({ block: 'nearest' });
  show();
}

// Moves the picked page `by` places later (earlier when negative), within the list.
// Moving it takes the focus away, so the keys give it back.
function move(by, focus) {
  if (!picked) return;
  const all = tiles();
  const to = Math.max(0, Math.min(all.length - 1, all.indexOf(picked) + by));
  const rest = all.filter((li) => li !== picked);
  list.insertBefore(picked, rest[to] ?? null);
  pick(picked, focus);
}

// How many pages fit in a row, for the up and down keys.
function columns() {
  const all = tiles();
  const top = all[0]?.offsetTop;
  const n = all.findIndex((li) => li.offsetTop !== top);
  return n < 0 ? all.length : n;
}

function step(key) {
  if (key === 'ArrowLeft') return -1;
  if (key === 'ArrowRight') return 1;
  if (key === 'ArrowUp') return -columns();
  if (key === 'ArrowDown') return columns();
  return 0;
}

function lift() {
  const box = drag.li.getBoundingClientRect();
  const ghost = drag.li.cloneNode(true);
  ghost.className = 'ghost';
  ghost.setAttribute('aria-hidden', 'true');
  ghost.style.width = `${box.width}px`;
  ghost.style.height = `${box.height}px`;
  document.body.append(ghost);
  drag.li.classList.add('lifted');
  drag.ghost = ghost;
  drag.frame = requestAnimationFrame(scroll);
}

// The copy goes with the pointer, and the page moves to the place under it: before
// the page there, or after it on that page's right half.
function follow() {
  drag.ghost.style.transform = `translate(${drag.x - drag.dx}px, ${drag.y - drag.dy}px)`;
  const over = document.elementFromPoint(drag.x, drag.y)?.closest('#pages > li');
  if (!over || over === drag.li) return;
  const box = over.getBoundingClientRect();
  list.insertBefore(drag.li, drag.x > box.left + box.width / 2 ? over.nextSibling : over);
  show();
}

// Near the top or bottom of the stage the list scrolls, faster nearer the edge.
function scroll() {
  if (!drag) return;
  const box = list.getBoundingClientRect();
  let by = 0;
  if (drag.y < box.top + EDGE) by = drag.y - (box.top + EDGE);
  else if (drag.y > box.bottom - EDGE) by = drag.y - (box.bottom - EDGE);
  if (by) {
    list.scrollTop += Math.max(-EDGE, Math.min(EDGE, by)) / 3;
    follow();
  }
  drag.frame = requestAnimationFrame(scroll);
}

function drop() {
  if (!drag) return;
  if (drag.ghost) {
    cancelAnimationFrame(drag.frame);
    drag.ghost.remove();
    drag.li.classList.remove('lifted');
  }
  drag = null;
  show();
}

list.addEventListener('pointerdown', (event) => {
  const li = event.target.closest('#pages > li');
  if (!li || event.button !== 0 || drag) return;
  event.preventDefault();
  pick(li);
  const box = li.getBoundingClientRect();
  const [x, y] = [event.clientX, event.clientY];
  drag = { li, id: event.pointerId, startX: x, startY: y, x, y, dx: x - box.left, dy: y - box.top, ghost: null, frame: 0 };
  // On the list, which stays put: the page itself moves around in it.
  list.setPointerCapture(event.pointerId);
});

list.addEventListener('pointermove', (event) => {
  if (!drag || event.pointerId !== drag.id) return;
  drag.x = event.clientX;
  drag.y = event.clientY;
  if (!drag.ghost) {
    if (Math.hypot(drag.x - drag.startX, drag.y - drag.startY) < DRAG_START) return;
    lift();
  }
  follow();
});

list.addEventListener('pointerup', drop);
list.addEventListener('pointercancel', drop);
list.addEventListener('lostpointercapture', drop);

async function reorder() {
  if (!session || apply.disabled) return;
  apply.disabled = true;
  try {
    // The app checks the order, queues the job and closes this window.
    await invoke('editor_order', { order: order() });
  } catch (error) {
    say(String(error));
    show();
  }
}

function cancel() {
  invoke('editor_cancel').catch((e) => console.error('editor_cancel failed', e));
}

$('earlier').addEventListener('click', () => move(-1, false));
$('later').addEventListener('click', () => move(1, false));
$('reverse').addEventListener('click', () => {
  list.append(...tiles().reverse());
  if (picked) picked.scrollIntoView({ block: 'nearest' });
  show();
});
$('reset').addEventListener('click', () => {
  list.append(...tiles().sort((a, b) => a.dataset.page - b.dataset.page));
  if (picked) picked.scrollIntoView({ block: 'nearest' });
  show();
});
apply.addEventListener('click', reorder);
$('cancel').addEventListener('click', cancel);

document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') {
    cancel();
  } else if (event.key === 'Enter' && !(document.activeElement instanceof HTMLButtonElement)) {
    event.preventDefault();
    reorder();
  } else if (picked && !drag && list.contains(document.activeElement)) {
    const by = step(event.key);
    if (!by) return;
    event.preventDefault();
    if (event.shiftKey) {
      move(by, true);
    } else {
      const all = tiles();
      pick(all[Math.max(0, Math.min(all.length - 1, all.indexOf(picked) + by))]);
    }
  }
});

function build() {
  list.replaceChildren(...session.pages.map((src, i) => {
    const li = document.createElement('li');
    li.setAttribute('role', 'option');
    li.setAttribute('aria-selected', 'false');
    li.tabIndex = -1;
    li.dataset.page = String(i + 1);
    const img = document.createElement('img');
    img.src = src;
    img.alt = '';
    img.draggable = false;
    const number = document.createElement('span');
    number.textContent = String(i + 1);
    li.append(img, number);
    return li;
  }));
}

async function load() {
  try {
    session = await invoke('editor_session');
  } catch (error) {
    console.error('editor_session failed', error);
    session = null;
  }
  const valid = session
    && session.kind === 'order'
    && Array.isArray(session.pages) && session.pages.length > 1
    && session.pages.every((p) => typeof p === 'string' && p.startsWith('data:image/png;base64,'));
  if (!valid) {
    session = null;
    show();
    say('Nothing to reorder. Drag the PDF onto Reorder again.');
    return;
  }
  document.title = `Reorder: ${session.name}`;
  build();
  pick(list.firstElementChild);
}

load();
