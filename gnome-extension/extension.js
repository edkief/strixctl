// SPDX-License-Identifier: GPL-2.0-or-later
import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
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
    <property name="CpuFreqMhz" type="u" access="read"/>
    <property name="PowerDrawW" type="d" access="read"/>
    <property name="BatteryMinutesLeft" type="i" access="read"/>
    <property name="CurrentPlatformProfile" type="s" access="read"/>
    <property name="CurrentSavedProfile" type="s" access="read"/>
    <method name="NotifyProfileApplied">
      <arg type="s" direction="in" name="name"/>
    </method>
    <signal name="TempChanged">
      <arg type="d" name="temp"/>
    </signal>
    <signal name="PlatformProfileChanged">
      <arg type="s" name="profile"/>
    </signal>
    <signal name="SavedProfileChanged">
      <arg type="s" name="name"/>
    </signal>
    <signal name="MetricsChanged">
      <arg type="u" name="freq_mhz"/>
      <arg type="d" name="power_w"/>
      <arg type="i" name="battery_minutes"/>
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
        this._platformProfile = proxy.CurrentPlatformProfile ?? null;
        this._savedProfile = proxy.CurrentSavedProfile || null;

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

        // Track platform profile changes from any source (GUI, CLI, etc.).
        this._profileSignalId = this._proxy.connectSignal(
            'PlatformProfileChanged',
            (_proxy, _sender, [profile]) => {
                this._platformProfile = profile;
                this._updateSubtitle();
            }
        );

        // Track saved profile changes — preferred over platform profile in subtitle.
        this._savedProfileSignalId = this._proxy.connectSignal(
            'SavedProfileChanged',
            (_proxy, _sender, [name]) => {
                this._savedProfile = name || null;
                this._updateSubtitle();
            }
        );

        this._refreshProfiles(/* applyOnLoad= */ true);

        this.connect('button-press-event', (_toggle, event) => this._handleClick(event));
    }

    _updateSubtitle() {
        const parts = [];
        const profileLabel = this._savedProfile || this._platformProfile;
        if (profileLabel)
            parts.push(profileLabel);
        if (this._tempC !== null && this._tempC > 0)
            parts.push(`${this._tempC.toFixed(1)} °C`);
        this.subtitle = parts.length ? parts.join('  ·  ') : null;
    }

    _refreshProfiles(applyOnLoad = false) {
        this._proxy.ListSavedProfilesRemote((result, error) => {
            if (error) {
                this._profiles = [];
            } else {
                this._profiles = result[0] ?? [];
            }
            this._buildMenu();

            if (applyOnLoad) {
                const saved = this._proxy.CurrentSavedProfile;
                if (saved)
                    this._applyProfile(saved, /* silent= */ true);
            }
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
        } else {
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

        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());

        const launchItem = new PopupMenu.PopupMenuItem('Open strixctl');
        launchItem.connect('activate', () => {
            Shell.AppSystem.get_default().lookup_app('strixctl.desktop')?.activate();
        });
        this.menu.addMenuItem(launchItem);
    }

    _applyProfile(name, silent = false) {
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
            this._savedProfile = name;
            this._updateSubtitle();
            this._buildMenu();
            if (!silent)
                _showProfileOSD(name);
        });
    }

    _handleClick(event) {
        const button = event.get_button();
        // _activeProfile is only set when the user applies via this session's UI;
        // fall back to _savedProfile (from D-Bus property) so right-click works
        // correctly after a shell restart.
        const target = this._activeProfile ?? this._savedProfile;
        if (button === 3 && target) {
            this._applyProfile(target);
        }
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
        if (this._profileSignalId !== undefined) {
            this._proxy.disconnectSignal(this._profileSignalId);
            this._profileSignalId = undefined;
        }
        if (this._savedProfileSignalId !== undefined) {
            this._proxy.disconnectSignal(this._savedProfileSignalId);
            this._savedProfileSignalId = undefined;
        }
        super.destroy();
    }
});

// ── Panel pill (metrics next to the clock) ───────────────────────────────
//
// A plain label at the left of the panel's right-hand box. What it shows
// is chosen in prefs: CPU frequency, battery power draw, remaining battery time,
// or any combination. Values arrive through the daemon's MetricsChanged signal
// (at most 1 Hz), with the D-Bus properties used for the initial paint.

const StrixCtlPill = GObject.registerClass(
class StrixCtlPill extends PanelMenu.Button {
    _init(proxy, settings) {
        // dontCreateMenu: this is a readout, not a menu button.
        super._init(0.5, 'strixctl', true);

        this._proxy = proxy;
        this._settings = settings;

        this._freqMhz = proxy.CpuFreqMhz ?? 0;
        this._powerW = proxy.PowerDrawW ?? 0;
        this._batteryMinutes = proxy.BatteryMinutesLeft ?? -1;

        this._label = new St.Label({
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'strixctl-pill-label',
        });
        this.add_child(this._label);

        this._metricsSignalId = this._proxy.connectSignal(
            'MetricsChanged',
            (_proxy, _sender, [freqMhz, powerW, batteryMinutes]) => {
                this._freqMhz = freqMhz;
                this._powerW = powerW;
                this._batteryMinutes = batteryMinutes;
                this._updateLabel();
            }
        );

        this._settingsIds = [
            'pill-enabled',
            'pill-show-freq',
            'pill-show-power',
            'pill-show-battery',
        ].map(key => this._settings.connect(`changed::${key}`, () => this._updateLabel()));

        this.connect('button-press-event', () => {
            Shell.AppSystem.get_default().lookup_app('strixctl.desktop')?.activate();
            return Clutter.EVENT_STOP;
        });

        this._updateLabel();
    }

    _updateLabel() {
        const parts = [];
        if (this._settings.get_boolean('pill-show-freq'))
            parts.push(this._freqMhz > 0 ? `${(this._freqMhz / 1000).toFixed(2)} GHz` : '— GHz');
        if (this._settings.get_boolean('pill-show-power'))
            parts.push(this._powerW > 0 ? `${this._powerW.toFixed(1)} W` : 'AC');
        if (this._settings.get_boolean('pill-show-battery')) {
            if (this._batteryMinutes >= 0) {
                const h = Math.floor(this._batteryMinutes / 60);
                const m = this._batteryMinutes % 60;
                parts.push(`${h}:${m.toString().padStart(2, '0')}`);
            } else {
                parts.push('AC');
            }
        }

        // Nothing selected (or the pill switched off) means no panel real estate.
        const visible = this._settings.get_boolean('pill-enabled') && parts.length > 0;
        this.visible = visible;
        if (visible)
            this._label.text = parts.join('  ·  ');
    }

    destroy() {
        if (this._metricsSignalId !== undefined) {
            this._proxy.disconnectSignal(this._metricsSignalId);
            this._metricsSignalId = undefined;
        }
        for (const id of this._settingsIds ?? [])
            this._settings.disconnect(id);
        this._settingsIds = [];
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

        // Panel pill, at the left edge of the right-hand status area — right
        // next to the system indicators rather than beside the clock.
        this._settings = this.getSettings();
        this._pill = new StrixCtlPill(this._proxy, this._settings);
        Main.panel.addToStatusArea('strixctl-pill', this._pill, 0, 'right');

        // Hotkey: default <Super>p, configurable in prefs or via gsettings.
        // On ASUS ROG laptops, Fn+Q may report as XF86Launch1 — set that in prefs.
        Main.wm.addKeybinding(
            'cycle-profiles-key',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this._indicator._toggle.cycleNext()
        );
    }

    disable() {
        Main.wm.removeKeybinding('cycle-profiles-key');
        this._pill?.destroy();
        this._pill = null;
        this._settings = null;
        this._indicator?.destroy();
        this._indicator = null;
        this._proxy = null;
    }
}
