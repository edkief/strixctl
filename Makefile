install-polkit:
	install -Dm644 polkit/com.strixctrl.ryzenadj.policy \
	  /usr/share/polkit-1/actions/com.strixctrl.ryzenadj.policy

uninstall-polkit:
	rm -f /usr/share/polkit-1/actions/com.strixctrl.ryzenadj.policy

.PHONY: install-polkit uninstall-polkit
