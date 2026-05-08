// SPDX-License-Identifier: GPL-2.0-or-later
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const DBUS_NAME = 'com.strixctl.Service';
const DBUS_PATH = '/com/strixctl/Service';

const IFACE_XML = `<node>
  <interface name="com.strixctl.Service">
    <method name="ListSavedProfiles">
      <arg type="as" direction="out" name="profiles"/>
    </method>
    <method name="ApplySavedProfile">
      <arg type="s" direction="in" name="name"/>
    </method>
    <method name="SetAsusProfile">
      <arg type="s" direction="in" name="profile"/>
    </method>
    <property name="CurrentTempC" type="d" access="read"/>
    <signal name="TempChanged">
      <arg type="d" name="temp"/>
    </signal>
  </interface>
</node>`;

const StrixCtlProxy = Gio.DBusProxy.makeProxyWrapper(IFACE_XML);

// ── Quick Settings toggle ─────────────────────────────────────────────────

const StrixCtlToggle = GObject.registerClass(
class StrixCtlToggle extends QuickSettings.QuickMenuToggle {
    _init(proxy) {
        super._init({
            title: 'Profiles',
            iconName: 'power-profile-performance-symbolic',
            toggleMode: false,
        });

        this._proxy = proxy;
        this._profiles = [];
        this._activeProfile = null;
        this._tempC = proxy.CurrentTempC ?? null;

        this.menu.setHeader('power-profile-performance-symbolic', 'strixctl');
        this._updateSubtitle();

        // Refresh profile list each time the menu is opened.
        this.menu.connect('open-state-changed', (_menu, open) => {
            if (open)
                this._refreshProfiles();
        });

        // Track temperature changes for the subtitle.
        this._tempSignalId = this._proxy.connectSignal(
            'TempChanged',
            (_proxy, _sender, [temp]) => {
                this._tempC = temp;
                this._updateSubtitle();
            }
        );

        this._refreshProfiles();
    }

    _updateSubtitle() {
        const parts = [];
        if (this._activeProfile !== null)
            parts.push(this._activeProfile);
        if (this._tempC !== null && this._tempC > 0)
            parts.push(`${this._tempC.toFixed(1)} °C`);
        this.subtitle = parts.length ? parts.join('  ·  ') : null;
    }

    _refreshProfiles() {
        this._proxy.ListSavedProfilesRemote((result, error) => {
            if (error) {
                this._profiles = [];
            } else {
                this._profiles = result[0] ?? [];
            }
            this._buildMenu();
        });
    }

    _buildMenu() {
        this.menu.removeAll();
        this.menu.setHeader('power-profile-performance-symbolic', 'strixctl Profiles');

        if (!this._profiles.length) {
            const placeholder = new PopupMenu.PopupMenuItem(
                'No saved profiles', {reactive: false}
            );
            this.menu.addMenuItem(placeholder);
            return;
        }

        for (const name of this._profiles) {
            const item = new PopupMenu.PopupMenuItem(name);
            item.setOrnament(
                name === this._activeProfile
                    ? PopupMenu.Ornament.CHECK
                    : PopupMenu.Ornament.NONE
            );
            item.connect('activate', () => this._applyProfile(name));
            this.menu.addMenuItem(item);
        }
    }

    _applyProfile(name) {
        this._proxy.ApplySavedProfileRemote(name, (_result, error) => {
            if (error) {
                // Pull the readable part out of the D-Bus error string.
                // zbus wraps the message as "GDBus.Error:com.…Failed: <text>"
                const raw = error.message ?? String(error);
                const detail = raw.replace(/^.*?:\s*/, '');
                this._lastError = detail;
                this.subtitle = `⚠ ${name}`;
                Main.notify('strixctl — apply failed', detail);
                // Rebuild so the checkmark reflects unchanged state.
                this._buildMenu();
                return;
            }
            this._lastError = null;
            this._activeProfile = name;
            this._updateSubtitle();
            this._buildMenu();
            _showProfileOSD(name);
        });
    }

    // Called by the extension keybinding handler.
    cycleNext() {
        if (!this._profiles.length)
            return;
        const idx = this._activeProfile !== null
            ? this._profiles.indexOf(this._activeProfile)
            : -1;
        const next = this._profiles[(idx + 1) % this._profiles.length];
        this._applyProfile(next);
    }

    destroy() {
        if (this._tempSignalId !== undefined) {
            this._proxy.disconnectSignal(this._tempSignalId);
            this._tempSignalId = undefined;
        }
        super.destroy();
    }
});

// ── System indicator (the icon that sits in the status bar) ──────────────

const StrixCtlIndicator = GObject.registerClass(
class StrixCtlIndicator extends QuickSettings.SystemIndicator {
    _init(proxy) {
        super._init();
        this._toggle = new StrixCtlToggle(proxy);
        this.quickSettingsItems.push(this._toggle);
    }

    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        super.destroy();
    }
});

// ── OSD popup shown on profile switch ────────────────────────────────────

function _showProfileOSD(label) {
    try {
        Main.osdWindowManager.showOne(
            0,                                                   // primary monitor
            Gio.Icon.new_for_string('power-profile-performance-symbolic'),
            label,
            null,                                                // no progress value
            false                                                // no level bar
        );
    } catch (e) {
        logError(e, 'strixctl: OSD');
    }
}

// ── Extension entry point ─────────────────────────────────────────────────

export default class StrixCtlExtension extends Extension {
    enable() {
        // Proxy is created synchronously; D-Bus activation fires on first method call.
        this._proxy = new StrixCtlProxy(
            Gio.DBus.session,
            DBUS_NAME,
            DBUS_PATH,
            null,
            null,
            Gio.DBusProxyFlags.DO_NOT_AUTO_START_AT_CONSTRUCTION
        );

        this._indicator = new StrixCtlIndicator(this._proxy);
        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator);

        // Hotkey: default <Super>p, configurable in prefs or via gsettings.
        // On ASUS ROG laptops, Fn+Q may report as XF86Launch1 — set that in prefs.
        Main.wm.addKeybinding(
            'cycle-profiles-key',
            this.getSettings(),
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._indicator._toggle.cycleNext()
        );
    }

    disable() {
        Main.wm.removeKeybinding('cycle-profiles-key');
        this._indicator?.destroy();
        this._indicator = null;
        this._proxy = null;
    }
}
