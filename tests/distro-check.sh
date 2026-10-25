#!/bin/sh
# Inside a distribution's container image: does `mica` identify the
# distribution and its package manager, and does every package `mica setup`
# would install exist in that distribution's repositories?
#
#     docker run --rm -v "$PWD:/w" -w /w IMAGE sh tests/distro-check.sh MICA FAMILY MANAGER
#
# MICA is a static Linux build of mica, FAMILY the expected distribution
# family (debian, fedora, arch, suse, nix), and MANAGER the expected package
# manager (apt, dnf, pacman, zypper, nix). Prints a Markdown report.

set -eu
mica=$1
family=$2
manager=$3

echo "### $(. /etc/os-release && echo "$PRETTY_NAME")"
echo
"$mica" doctor
"$mica" doctor --json > /tmp/doctor.json
grep -q "\"family\": \"$family\"" /tmp/doctor.json || { echo "FAIL: expected family $family"; exit 1; }
grep -q "\"$manager\"" /tmp/doctor.json || { echo "FAIL: expected package manager $manager"; exit 1; }
echo "identified: family $family, package manager $manager"

"$mica" setup windows vulkan vm security --json > /tmp/plans.json
# Package names, one per line, from the plans' "packages" arrays.
packages=$(tr -d '\n' < /tmp/plans.json | grep -o '"packages": *\[[^]]*\]' | grep -o '"[^"]*"' | grep -v '"packages"' | tr -d '"')

missing=""
case "$manager" in
    apt)
        apt-get update -qq >/dev/null
        for package in $packages; do
            apt-cache show "$package" >/dev/null 2>&1 || missing="$missing $package"
        done
        ;;
    dnf)
        for package in $packages; do
            case "$package" in
                @*) dnf -q group info "${package#@}" >/dev/null 2>&1 || missing="$missing $package" ;;
                *) dnf -q info "$package" >/dev/null 2>&1 || missing="$missing $package" ;;
            esac
        done
        ;;
    pacman)
        # wine lives in [multilib], which Arch ships disabled.
        printf '\n[multilib]\nInclude = /etc/pacman.d/mirrorlist\n' >> /etc/pacman.conf
        pacman -Sy --noconfirm >/dev/null
        for package in $packages; do
            pacman -Si "$package" >/dev/null 2>&1 || missing="$missing $package"
        done
        ;;
    zypper)
        zypper --non-interactive --quiet refresh >/dev/null
        for package in $packages; do
            zypper --non-interactive info "$package" 2>/dev/null | grep -q "^Name" || missing="$missing $package"
        done
        ;;
    nix)
        packages=""
        echo "NixOS: mica prints configuration.nix instructions instead of installing"
        ;;
esac
echo "packages checked: $(echo $packages | wc -w)"
if [ -n "$missing" ]; then
    echo "FAIL: not in the repositories:$missing"
    exit 1
fi
echo "every package mica would install exists"
