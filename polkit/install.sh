#!/usr/bin/env bash
set -e
cd "$(dirname "$0")"
install -m 644 com.strixctl.ryzenadj.policy /usr/share/polkit-1/actions/
echo "Policy installed. No restart required."
