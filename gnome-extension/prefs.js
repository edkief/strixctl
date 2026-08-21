// SPDX-License-Identifier: GPL-2.0-or-later
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';
import Gtk from 'gi://Gtk';
import Adw from 'gi://Adw';
// Prefs run in the gnome-extensions prefs process, not in gnome-shell, so this
// comes from the org.gnome.Shell.Extensions gresource — note the capital S and
// the js/extensions/ segment. The lowercase shell/extensions/ path only exists
// inside the shell process (that's where extension.js gets its imports).
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class StrixCtrlPrefs extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();

        const page = new Adw.PreferencesPage({
            title: 'General',
            iconName: 'preferences-system-symbolic',
        });
        window.add(page);

        // ── Keybinding ────────────────────────────────────────────────────

        const kbGroup = new Adw.PreferencesGroup({
            title: 'Keyboard Shortcut',
            description: 'Cycles to the next saved profile in sequence.',
        });
        page.add(kbGroup);

        const kbRow = new Adw.ActionRow({title: 'Cycle profiles'});
        kbGroup.add(kbRow);

        const shortcutLabel = new Gtk.ShortcutLabel({
            accelerator: settings.get_strv('cycle-profiles-key')[0] ?? '',
            disabled_text: 'Disabled',
            valign: Gtk.Align.CENTER,
        });
        settings.connect('changed::cycle-profiles-key', () => {
            shortcutLabel.set_accelerator(
                settings.get_strv('cycle-profiles-key')[0] ?? ''
            );
        });

        const editBtn = new Gtk.Button({
            icon_name: 'document-edit-symbolic',
            valign: Gtk.Align.CENTER,
            css_classes: ['flat'],
            tooltip_text: 'Click, then press your shortcut',
        });

        // Simple capture: open a dialog that records the next key press.
        editBtn.connect('clicked', () => {
            const dialog = new Adw.MessageDialog({
                transient_for: window,
                heading: 'Press shortcut',
                body: 'Press the key combination you want to use, or Escape to cancel.',
            });
            dialog.add_response('cancel', 'Cancel');
            dialog.set_default_response('cancel');

            const controller = new Gtk.EventControllerKey();
            controller.connect('key-pressed', (_ctl, keyval, keycode, state) => {
                // Ignore bare modifier keys
                const mods = state & Gtk.accelerator_get_default_mod_mask();
                if (Gtk.accelerator_valid(keyval, mods)) {
                    const accel = Gtk.accelerator_name(keyval, mods);
                    settings.set_strv('cycle-profiles-key', [accel]);
                    dialog.close();
                    return true;
                }
                if (keyval === 65307) { // Escape
                    dialog.close();
                    return true;
                }
                return false;
            });
            dialog.add_controller(controller);
            dialog.present();
        });

        const clearBtn = new Gtk.Button({
            icon_name: 'edit-clear-symbolic',
            valign: Gtk.Align.CENTER,
            css_classes: ['flat'],
            tooltip_text: 'Clear shortcut',
        });
        clearBtn.connect('clicked', () => settings.set_strv('cycle-profiles-key', []));

        kbRow.add_suffix(shortcutLabel);
        kbRow.add_suffix(editBtn);
        kbRow.add_suffix(clearBtn);

        // ── Panel pill ────────────────────────────────────────────────────

        const pillGroup = new Adw.PreferencesGroup({
            title: 'Panel Pill',
            description: 'A live readout at the right of the top bar. ' +
                'It needs the strixctld daemon running.',
        });
        page.add(pillGroup);

        const enabledRow = new Adw.SwitchRow({
            title: 'Show pill',
            subtitle: 'Hidden automatically when nothing below is selected.',
        });
        pillGroup.add(enabledRow);
        settings.bind('pill-enabled', enabledRow, 'active',
            Gio.SettingsBindFlags.DEFAULT);

        const rows = [
            ['pill-show-freq', 'CPU frequency', 'Fastest live core, in GHz.'],
            ['pill-show-power', 'Power draw', 'Battery discharge in watts, "AC" on mains.'],
            ['pill-show-battery', 'Battery time left', 'Estimated h:mm at the current draw.'],
        ];
        for (const [key, title, subtitle] of rows) {
            const row = new Adw.SwitchRow({title, subtitle});
            pillGroup.add(row);
            settings.bind(key, row, 'active', Gio.SettingsBindFlags.DEFAULT);
            // The contents only matter while the pill itself is shown.
            enabledRow.bind_property('active', row, 'sensitive',
                GObject.BindingFlags.SYNC_CREATE);
        }

        // ── Fn+Q note ─────────────────────────────────────────────────────

        const noteGroup = new Adw.PreferencesGroup({title: 'ASUS ROG tip'});
        page.add(noteGroup);

        const noteRow = new Adw.ActionRow({
            title: 'Using Fn+Q',
            subtitle: 'If Fn+Q is reported to GNOME as XF86Launch1, set the shortcut above to that key. Run "xev" in a terminal and press Fn+Q to confirm the keycode.',
        });
        noteGroup.add(noteRow);
    }
}
