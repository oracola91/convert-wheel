// The Settings window. The app hands over everything as it is now (settings_view);
// each change goes back as one small message (settings_change) and the app answers
// with the new state, which is drawn again. Nothing is kept here.
if (!window.__TAURI__) throw new Error('Pirouette: the Tauri bridge is missing');
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const TABS = ['general', 'wheel', 'quality', 'about'];
let view = null;
let tab = TABS.includes(location.hash.slice(1)) ? location.hash.slice(1) : 'general';
let updating = false;
// Install Missing Converters… is running (the button says so and waits).
let installing = false;

// A small element builder: h('div', { class: 'row' }, child, 'text', ...).
function h(tag, attrs = {}, ...children) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === false || v == null) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (v === true) el.setAttribute(k, '');
    else el.setAttribute(k, v);
  }
  for (const c of children.flat()) {
    if (c == null || c === false) continue;
    el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  }
  return el;
}

// One line: its name (a real label for a switch or a list, so screen readers say it),
// a note under the name, and the control on the right.
function row(label, control, note, stack) {
  const named = control.id && ['INPUT', 'SELECT'].includes(control.tagName);
  const name = named ? h('label', { for: control.id }, label) : label;
  return h('div', { class: stack ? 'row stack' : 'row' }, h('div', { class: 'label' }, name, note ? h('span', { class: 'note' }, note) : null), control);
}

function group(title, ...rows) {
  return [title ? h('h2', {}, title) : null, h('div', { class: 'group' }, ...rows)];
}

function toggle(id, on, onchange, disabled) {
  return h('input', { type: 'checkbox', class: 'switch', id, role: 'switch', checked: on, disabled, onchange });
}

// A choice of a few: radio buttons drawn as a segmented control.
function segments(name, choices, picked, onpick, label) {
  return h(
    'div',
    { class: 'segments', role: 'radiogroup', 'aria-label': label },
    choices.map((c) =>
      h('label', {}, h('input', { type: 'radio', name, id: `${name}-${c.key}`, value: c.key, checked: c.key === picked, onchange: () => onpick(c.key) }), c.label),
    ),
  );
}

async function change(c) {
  try {
    view = await invoke('settings_change', { change: c });
    showProblem('');
  } catch (e) {
    showProblem(String(e));
  }
  draw();
}

function showProblem(text) {
  $('problem').textContent = text;
  $('problem').hidden = !text;
}

function keyName(view, key) {
  return view.combos.find((c) => c.key === key)?.label ?? key;
}

function general(v) {
  const login = toggle('open-at-login', v.login.on, (e) => change({ what: 'login', on: e.target.checked }), !v.login.available);
  const rows = [
    row('Open at login', login, v.login.available ? null : 'Only for the installed app.'),
    row(
      'Save converted files',
      segments('save', [{ key: 'next-to-original', label: 'Next to the original' }, { key: 'downloads', label: 'In Downloads' }], v.save_to, (to) => change({ what: 'save-to', to }), 'Save converted files'),
    ),
  ];
  if (v.shelf !== null) {
    rows.push(row('Shelf at the top of the screen', toggle('shelf', v.shelf, (e) => change({ what: 'shelf', on: e.target.checked })), 'Holds files you drag to the top of the screen, to drag out later.'));
  }
  // The keyboard wheel: how each system hands it the selected files.
  const entry = {
    macos: ['Finder: right-click > Services > Open in Pirouette Wheel', 'Set a key for it in System Settings > Keyboard > Keyboard Shortcuts > Services. Then ← → pick, Enter applies, Esc closes.'],
    windows: ['File Explorer: right-click > Send to > Convert Wheel', 'Then ← → pick, Enter applies, Esc closes.'],
    linux: ['Files: right-click > Open With > Convert Wheel', 'Then ← → pick, Enter applies, Esc closes.'],
  }[v.system];
  const keyboardRow = v.send_to !== null
    ? row(entry[0], toggle('send-to', v.send_to, (e) => change({ what: 'send-to', on: e.target.checked })), entry[1])
    : row(entry[0], h('span', { class: 'note' }, ''), entry[1]);
  const parts = [
    h('p', { class: 'lead' }, `Hold ${keyName(v, 'shift')} while dragging a file to open the wheel.`),
    group('', ...rows),
    group('Open the wheel from the keyboard', keyboardRow),
  ];
  if (v.converters) {
    const button = h(
      'button',
      {
        type: 'button',
        class: 'action',
        id: 'update-converters',
        // One install or update at a time: the other button waits too.
        disabled: updating || installing || v.busy,
        onclick: () => {
          updating = true;
          draw();
          invoke('settings_action', { action: 'update-converters' }).catch((e) => {
            updating = false;
            showProblem(String(e));
            draw();
          });
        },
      },
      updating ? 'Updating…' : 'Update Converters…',
    );
    const names = (list) => (list.length > 1 ? `${list.slice(0, -1).join(', ')} and ${list[list.length - 1]}` : list[0]);
    const installRow = v.missing.length > 0
      ? row(
        `Not installed: ${names(v.missing)}`,
        h(
          'button',
          {
            type: 'button',
            class: 'action',
            id: 'install-all',
            disabled: installing || updating || v.busy,
            onclick: () => {
              installing = true;
              draw();
              invoke('settings_action', { action: 'install-all' }).catch((e) => {
                installing = false;
                showProblem(String(e));
                draw();
              });
            },
          },
          installing ? 'Installing…' : 'Install Missing Converters…',
        ),
        'All of them in one go (LibreOffice is a large download). A notification says when they are in.',
      )
      : row('All converters are installed', h('span', { class: 'note' }, ''));
    parts.push(group('Converters', installRow, row('Newer versions of ffmpeg, ImageMagick and the others', button, 'Only those the package manager installed. A notification says what changed.')));
  }
  return parts;
}

function wheel(v) {
  const offAndCombos = () => [...v.combos.map((c) => ({ key: c.key, label: c.label })), { key: 'off', label: 'Off' }];
  const keyRows = v.keys.map((k) =>
    row(
      k.label,
      h(
        'select',
        { id: `keys-${k.wheel}`, onchange: (e) => change({ what: 'keys', wheel: k.wheel, combo: e.target.value === 'off' ? null : e.target.value }) },
        offAndCombos().map((c) => h('option', { value: c.key, selected: (k.combo ?? 'off') === c.key }, c.label)),
      ),
    ),
  );
  if (v.arrows !== null) {
    keyRows.push(
      row('Arrow keys on the wheel', toggle('arrow-keys', v.arrows, (e) => change({ what: 'arrows', on: e.target.checked })), 'Left and Right pick a segment, Up and Down show the next set; drop on the middle. macOS asks for Input Monitoring when you switch this on.'),
    );
  } else {
    keyRows.push(row('Arrow keys on the wheel', h('span', { class: 'note' }, 'Always on'), 'Left and Right pick a segment, Up and Down show the next set; drop on the middle.'));
  }

  const swatches = h(
    'div',
    { class: 'swatches', role: 'radiogroup', 'aria-label': 'Highlight colour' },
    v.colors.map((c) =>
      h('label', { style: `background:${c.color}`, title: c.label }, h('input', { type: 'radio', name: 'highlight', id: `highlight-${c.key}`, value: c.key, 'aria-label': c.label, checked: c.key === v.highlight, onchange: () => change({ what: 'highlight', color: c.key }) })),
    ),
  );

  const mac = v.system === 'macos';
  const glass = toggle('glass', v.glass, (e) => change({ what: 'glass', on: e.target.checked }));
  const slider = h('input', {
    type: 'range',
    id: 'transparency',
    min: 0,
    max: 100,
    step: 5,
    value: v.transparency,
    disabled: !v.glass,
    'aria-label': mac ? 'Transparency, from frosted to clear' : 'Transparency, from less to more',
    // Saved when let go, as the wheel only takes it at its next opening.
    onchange: (e) => change({ what: 'transparency', value: Number(e.target.value) }),
  });
  const look = group(
    'Look',
    row('Highlight colour', swatches),
    row(mac ? 'Liquid Glass' : 'See-through', glass, mac ? 'Off: a plain solid wheel.' : 'Off: a solid wheel.'),
    row('Transparency', h('div', { class: 'ends' }, mac ? 'Frosted' : 'Less', slider, mac ? 'Clear' : 'More')),
  );

  const kinds = v.kinds.map((k) =>
    row(
      k.label,
      h(
        'div',
        { class: 'chips', role: 'group', 'aria-label': `${k.label} on the wheel` },
        k.formats.map((f) => h('label', {}, h('input', { type: 'checkbox', id: `format-${f.key}`, checked: f.shown, onchange: (e) => change({ what: 'format', key: f.key, shown: e.target.checked }) }), f.label)),
      ),
      null,
      true,
    ),
  );

  return [
    group('Keys', ...keyRows),
    h('p', { class: 'lead' }, 'Keys held with Shift while dragging. Keys another wheel had move to the one you set.'),
    look,
    group('Formats on the wheel', ...kinds),
  ];
}

function quality(v) {
  const rows = v.quality.map((q) => row(q.label, segments(`quality-${q.kind}`, v.levels, q.level, (level) => change({ what: 'quality', kind: q.kind, level }), q.label)));
  return [
    group('', ...rows),
    h('p', { class: 'lead' }, 'Lossless formats (PNG, TIFF, WAV, FLAC, AIFF) and documents have no quality to choose. The Tools wheel\'s Compress keeps its own, lower settings.'),
  ];
}

function about(v) {
  const log = h('button', { type: 'button', class: 'action', id: 'show-log', onclick: () => invoke('settings_action', { action: 'log' }).catch((e) => showProblem(String(e))) }, 'Show Log');
  return [h('h2', {}, ''), h('div', { class: 'group' }, h('h1', {}, v.app), h('p', {}, `Version ${v.version}`), log)];
}

const PANELS = { general, wheel, quality, about };

function draw() {
  if (!view) return;
  // Keep the keyboard where it was across the redraw.
  const focused = document.activeElement?.id;
  for (const name of TABS) {
    const panel = $(name);
    const on = name === tab;
    panel.hidden = !on;
    const button = $(`tab-${name}`);
    button.setAttribute('aria-selected', String(on));
    button.tabIndex = on ? 0 : -1;
    if (on) panel.replaceChildren(...PANELS[name](view).flat().filter(Boolean));
  }
  if (focused) $(focused)?.focus();
}

function pick(name, focus) {
  tab = name;
  history.replaceState(null, '', `#${name}`);
  draw();
  if (focus) $(`tab-${name}`).focus();
}

for (const name of TABS) {
  $(`tab-${name}`).addEventListener('click', () => pick(name));
}
// Arrow keys move between the tabs, as in any tab list.
$('tabs').addEventListener('keydown', (e) => {
  const step = { ArrowRight: 1, ArrowLeft: -1 }[e.key];
  if (!step) return;
  e.preventDefault();
  pick(TABS[(TABS.indexOf(tab) + step + TABS.length) % TABS.length], true);
});
// Esc closes the window; Cmd/Ctrl+W too.
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'w')) {
    e.preventDefault();
    invoke('settings_action', { action: 'close' }).catch(() => {});
  }
});

async function refresh() {
  try {
    view = await invoke('settings_view');
    draw();
  } catch (e) {
    showProblem(String(e));
  }
}

listen('converters-updating', (event) => {
  updating = Boolean(event.payload);
  // Done: look again, so the lists say what is there now.
  if (!updating) refresh();
  else if (tab === 'general') draw();
});
listen('converters-installing', (event) => {
  installing = Boolean(event.payload);
  // Done: "Not installed: …" goes, or keeps what failed, without switching tabs.
  if (!installing) refresh();
  else if (tab === 'general') draw();
});
// Open at Login can change in the system's own settings: look again on coming back.
window.addEventListener('focus', refresh);
refresh();
