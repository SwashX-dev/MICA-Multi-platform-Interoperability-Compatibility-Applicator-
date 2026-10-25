#!/bin/sh
# Remove MICA installed by scripts/install.sh.
#
#     sh scripts/uninstall.sh            # asks before deleting MICA's data
#
# 1. `mica uninstall` removes what MICA created (its Steam tool, its desktop
#    entry, and, if you agree, its prefixes, settings, and logs) and tells you
#    if a boot entry needs `sudo mica security boot-profile --rollback`;
# 2. then the `mica` program itself is deleted.

set -eu
INSTALL_DIR="${MICA_INSTALL_DIR:-$HOME/.local/bin}"
mica="$INSTALL_DIR/mica"

if [ -x "$mica" ]; then
    "$mica" uninstall || true
    rm -f "$mica"
    printf 'mica-uninstall: removed %s\n' "$mica"
elif command -v mica >/dev/null 2>&1; then
    printf 'mica-uninstall: mica at %s was not installed by install.sh;\n' "$(command -v mica)"
    printf 'mica-uninstall: remove it with the tool that installed it (apt, dnf, pacman, zypper, nix, cargo uninstall mica-cli)\n'
    exit 1
else
    printf 'mica-uninstall: mica is not installed in %s\n' "$INSTALL_DIR"
fi
