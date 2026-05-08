PREFIX ?= /usr/local

# Auto-detect whether sudo is needed; override with: make SUDO=doas all
ifeq ($(shell id -u),0)
SUDO :=
else
SUDO := sudo
endif

EXTENSION_UUID := strixctrl@strixctrl
EXTENSION_DIR  := $(HOME)/.local/share/gnome-shell/extensions/$(EXTENSION_UUID)
DBUS_USER_DIR  := $(HOME)/.local/share/dbus-1/services

# ── Build ─────────────────────────────────────────────────────────────────

.PHONY: build build-daemon

build:
	cargo build --release
	cargo build --release --features daemon --bin strixctrld

build-daemon:
	cargo build --release --features daemon --bin strixctrld

# ── System install (needs sudo) ───────────────────────────────────────────

.PHONY: install-polkit uninstall-polkit

install-polkit:
	$(SUDO) install -Dm644 polkit/com.strixctrl.ryzenadj.policy \
	  /usr/share/polkit-1/actions/com.strixctrl.ryzenadj.policy

uninstall-polkit:
	$(SUDO) rm -f /usr/share/polkit-1/actions/com.strixctrl.ryzenadj.policy

# ── User installs (no sudo) ───────────────────────────────────────────────

.PHONY: install-daemon install-systemd install-extension
.PHONY: uninstall-daemon uninstall-extension

# cargo install puts the binary in ~/.cargo/bin/, matching the ExecStart in
# systemd/strixctrld.service.  The D-Bus session activation file goes in the
# XDG user services dir so the session bus can auto-start the daemon.
install-daemon:
	cargo install --path . --features daemon --bin strixctrld
	install -d $(DBUS_USER_DIR)
	printf '[D-BUS Service]\nName=com.strixctrl.Service\nExec=%s/.cargo/bin/strixctrld\n' \
	    '$(HOME)' > $(DBUS_USER_DIR)/com.strixctrl.Service.service

uninstall-daemon:
	-cargo uninstall strixctrld
	rm -f $(DBUS_USER_DIR)/com.strixctrl.Service.service

install-systemd:
	install -Dm644 systemd/strixctrld.service \
	  $(HOME)/.config/systemd/user/strixctrld.service
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

all: install-polkit install-daemon install-systemd install-extension
	systemctl --user restart strixctrld || systemctl --user start strixctrld
	@echo ""
	@echo "Installation complete."
	@echo "Enable the GNOME extension with:"
	@echo "  gnome-extensions enable $(EXTENSION_UUID)"
	@echo "Reload GNOME Shell:  Alt+F2 → r  (X11)  or log out/in (Wayland)"

uninstall: uninstall-daemon uninstall-polkit uninstall-extension
	-systemctl --user stop    strixctrld
	-systemctl --user disable strixctrld
	rm -f $(HOME)/.config/systemd/user/strixctrld.service
	systemctl --user daemon-reload
	@echo "Uninstalled."
