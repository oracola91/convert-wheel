// Convert Wheel's helper inside GNOME Shell.
//
// On Wayland an app can't see another app's drag, the keys held during it or the
// pointer, so Convert Wheel can't notice a Shift+drag by itself. GNOME Shell can.
// While a drag goes on, this tells the app which keys are held, over the socket the
// app already listens on (src-tauri/src/linux.rs), and puts the wheel the app opens
// under the pointer: a Wayland app can't place its own windows.
//
// The app copies this file into place and keeps it up to date (gnome_shell.rs).

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

// How often the keys are sent while a drag goes on: the app's X11 poll.
const POLL_MS = 30;
// The wheel's window: its title and its size (src-tauri/tauri.conf.json).
const WHEEL_TITLE = 'Convert Wheel';
const WHEEL_SIZE = 340;

export default class ConvertWheelExtension extends Extension {
    enable() {
        this._handlers = [];
        this._displayHandlers = [];
        this._timer = 0;
        this._connection = null;
        this._seen = null;
        // Mutter's drag signals, as GNOME Shell's own xdndHandler.js uses them: "enter"
        // when a drag begins, "leave" when the button goes up.
        this._dnd = global.backend.get_dnd();
        this._handlers.push(this._dnd.connect('dnd-enter', () => this._begin()));
        this._handlers.push(this._dnd.connect('dnd-leave', () => this._end()));
        this._displayHandlers = [
            global.display.connect('window-created', (display, win) => this._onWindowCreated(win)),
            // Mutter doesn't give a window that opens during a drag the focus, and if it
            // overlaps the window that has it, asks for attention: GNOME Shell's "is
            // ready" notification. The wheel never wants the focus; this runs right
            // after the Shell's handler, so its notification goes before it is drawn.
            global.display.connect('window-demands-attention', (display, win) => {
                if (isWheel(win))
                    win.unset_demands_attention();
            }),
        ];
    }

    disable() {
        this._end();
        for (const id of this._handlers)
            this._dnd.disconnect(id);
        for (const id of this._displayHandlers)
            global.display.disconnect(id);
        this._handlers = [];
        this._displayHandlers = [];
        this._dnd = null;
    }

    // A wheel opening during a drag goes under the pointer as soon as Mutter shows it,
    // before it is drawn where Mutter put it.
    _onWindowCreated(win) {
        if (!this._connection || !isWheel(win))
            return;
        const id = win.connect('shown', () => {
            win.disconnect(id);
            this._place(win);
        });
    }

    // A drag began: tell the app which app it started in, then the keys until it ends.
    _begin() {
        this._end();
        this._connection = connect();
        if (!this._connection)
            return;
        const source = global.display.get_focus_window()?.get_wm_class() || '-';
        // A wheel already on screen (the keyboard shortcut's) stays where it is.
        this._seen = new Set(wheels());
        if (!this._send(`drag ${source.replace(/\s+/g, '_')}\n`))
            return;
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, POLL_MS, () => this._poll());
    }

    _poll() {
        const [, , mods] = global.get_pointer();
        const held = mask => (mods & mask ? '1' : '0');
        const keys = held(Clutter.ModifierType.SHIFT_MASK) +
            held(Clutter.ModifierType.MOD1_MASK) +
            held(Clutter.ModifierType.CONTROL_MASK);
        if (!this._send(`keys ${keys}\n`)) {
            this._timer = 0;
            return GLib.SOURCE_REMOVE;
        }
        // In case the wheel got its title only after Mutter created its window.
        for (const win of wheels())
            this._place(win);
        return GLib.SOURCE_CONTINUE;
    }

    _end() {
        if (this._timer) {
            GLib.source_remove(this._timer);
            this._timer = 0;
        }
        if (this._connection) {
            this._send('end\n');
            this._close();
        }
        this._seen = null;
    }

    // Writes one line to the app. Never waits, or the whole desktop would wait with
    // it: if the app isn't reading, the line is dropped and so is the connection.
    _send(line) {
        if (!this._connection)
            return false;
        try {
            const bytes = new TextEncoder().encode(line);
            const sent = this._connection.get_socket().send_with_blocking(bytes, false, null);
            if (sent === bytes.length)
                return true;
        } catch (e) {
            console.debug(`Convert Wheel: the app stopped listening (${e.message})`);
        }
        this._close();
        return false;
    }

    _close() {
        try {
            this._connection?.close(null);
        } catch {
            // Already closed.
        }
        this._connection = null;
    }

    // A wheel the app opened during this drag goes under the pointer, once, inside the
    // screen's work area, and above other windows (Mutter put it behind the window the
    // drag came from).
    _place(win) {
        if (!this._seen || this._seen.has(win))
            return;
        const frame = win.get_frame_rect();
        // Not on screen yet, or not the wheel after all.
        if (Math.abs(frame.width - WHEEL_SIZE) > 8 || Math.abs(frame.height - WHEEL_SIZE) > 8)
            return;
        this._seen.add(win);
        const [x, y] = global.get_pointer();
        const area = global.workspace_manager.get_active_workspace()
            .get_work_area_for_monitor(global.display.get_current_monitor());
        const within = (at, start, room) => Math.round(Math.min(Math.max(at, start), start + room));
        win.make_above();
        win.move_frame(true,
            within(x - frame.width / 2, area.x, area.width - frame.width),
            within(y - frame.height / 2, area.y, area.height - frame.height));
    }
}

function isWheel(win) {
    return win?.get_title() === WHEEL_TITLE;
}

// A connection to the app's socket, or null when the app isn't running.
function connect() {
    const path = GLib.build_filenamev([GLib.get_user_runtime_dir(), 'convert-wheel.sock']);
    try {
        return new Gio.SocketClient().connect(Gio.UnixSocketAddress.new(path), null);
    } catch {
        return null;
    }
}

// The app's wheel windows on screen.
function wheels() {
    return global.get_window_actors()
        .map(actor => actor.get_meta_window())
        .filter(isWheel);
}
