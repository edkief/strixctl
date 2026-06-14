PREFIX ?= /usr/local
PPT_DRIFT_GUARD ?= 0

# Auto-detect whether sudo is needed; override with: make SUDO=doas all
ifeq ($(shell id -u),0)
SUDO :=
else
SUDO := sudo
endif

EXTENSION_UUID := strixctl@strixctl
EXTENSION_DIR  := $(HOME)/.local/share/gnome-shell/extensions/$(EXTENSION_UUID)
DBUS_USER_DIR  := $(HOME)/.local/share/dbus-1/services

# ── Build ─────────────────────────────────────────────────────────────────

.PHONY: build build-daemon

DAEMON_FEATURES := daemon
ifeq ($(PPT_DRIFT_GUARD),1)
DAEMON_FEATURES := daemon,ppt-drift-guard
endif

build:
	cargo build --release
	cargo build --release --features "$(DAEMON_FEATURES)" --bin strixctld
	cargo build --release --bin strixctl-cpuctl

build-daemon:
	cargo build --release --features "$(DAEMON_FEATURES)" --bin strixctld

# ── System install (needs sudo) ───────────────────────────────────────────

.PHONY: install-polkit uninstall-polkit

install-polkit:
	$(SUDO) install -Dm644 polkit/com.strixctl.ryzenadj.policy \
	  /usr/share/polkit-1/actions/com.strixctl.ryzenadj.policy
	$(SUDO) install -Dm644 polkit/com.strixctl.cpuctl.policy \
	  /usr/share/polkit-1/actions/com.strixctl.cpuctl.policy

uninstall-polkit:
	$(SUDO) rm -f /usr/share/polkit-1/actions/com.strixctl.ryzenadj.policy
	$(SUDO) rm -f /usr/share/polkit-1/actions/com.strixctl.cpuctl.policy

# ── User installs (no sudo) ───────────────────────────────────────────────

.PHONY: install-bin install-desktop install-daemon install-systemd install-extension
.PHONY: uninstall-bin uninstall-desktop uninstall-daemon uninstall-extension

install-bin:
	$(SUDO) install -Dm755 target/release/strixctl $(PREFIX)/bin/strixctl
	$(SUDO) install -Dm755 target/release/strixctl-cpuctl $(PREFIX)/bin/strixctl-cpuctl

uninstall-bin:
	$(SUDO) rm -f $(PREFIX)/bin/strixctl
	$(SUDO) rm -f $(PREFIX)/bin/strixctl-cpuctl

install-desktop:
	$(SUDO) install -Dm644 strixctl.png $(PREFIX)/share/icons/hicolor/256x256/apps/strixctl.png
	$(SUDO) install -Dm644 strixctl.desktop $(PREFIX)/share/applications/strixctl.desktop
	-$(SUDO) gtk-update-icon-cache -f -t $(PREFIX)/share/icons/hicolor

uninstall-desktop:
	$(SUDO) rm -f $(PREFIX)/share/icons/hicolor/256x256/apps/strixctl.png
	$(SUDO) rm -f $(PREFIX)/share/applications/strixctl.desktop
	-$(SUDO) gtk-update-icon-cache -f -t $(PREFIX)/share/icons/hicolor

# cargo install puts the binary in ~/.cargo/bin/, matching the ExecStart in
# systemd/strixctld.service.  The D-Bus session activation file goes in the
# XDG user services dir so the session bus can auto-start the daemon.
install-daemon:
	install -Dm755 target/release/strixctld $(HOME)/.cargo/bin/strixctld
	install -d $(DBUS_USER_DIR)
	printf '[D-BUS Service]\nName=com.strixctl.Service\nExec=%s/.cargo/bin/strixctld\n' \
	    '$(HOME)' > $(DBUS_USER_DIR)/com.strixctl.Service.service

uninstall-daemon:
	-cargo uninstall strixctld
	rm -f $(DBUS_USER_DIR)/com.strixctl.Service.service

install-systemd:
	install -Dm644 systemd/strixctld.service \
	  $(HOME)/.config/systemd/user/strixctld.service
	systemctl --user daemon-reload

install-extension:
	install -d $(EXTENSION_DIR)/schemas
	install -m644 gnome-extension/metadata.json $(EXTENSION_DIR)/
	install -m644 gnome-extension/extension.js  $(EXTENSION_DIR)/
	install -m644 gnome-extension/prefs.js       $(EXTENSION_DIR)/
	install -m644 gnome-extension/schemas/*.xml  $(EXTENSION_DIR)/schemas/
	glib-compile-schemas $(EXTENSION_DIR)/schemas/

uninstall-extension:
	rm -rf $(EXTENSION_DIR)

# ── all / uninstall ───────────────────────────────────────────────────────

.PHONY: all uninstall

all: build install-polkit install-daemon install-systemd install-extension install-bin install-desktop
	systemctl --user restart strixctld || systemctl --user start strixctld
	@echo ""
	@echo "Installation complete."
	@echo "Enable the GNOME extension with:"
	@echo "  gnome-extensions enable $(EXTENSION_UUID)"
	@echo "Reload GNOME Shell:  Alt+F2 → r  (X11)  or log out/in (Wayland)"

uninstall: uninstall-bin uninstall-desktop uninstall-daemon uninstall-polkit uninstall-extension
	-systemctl --user stop    strixctld
	-systemctl --user disable strixctld
	rm -f $(HOME)/.config/systemd/user/strixctld.service
	systemctl --user daemon-reload
	@echo "Uninstalled."
