# RPM package for the `mica` command (Fedora, RHEL derivatives, openSUSE),
# built from a release tag with the distribution's Rust:
#
#     rpmbuild -ba --undefine=_disable_source_fetch packaging/rpm/mica.spec
#
# The build downloads crates from crates.io, so it suits local builds and
# COPR/OBS projects with network access rather than the Fedora repositories,
# whose guidelines require packaged crates.

%global repo MICA-Multi-platform-Interoperability-Compatibility-Applicator-
%global debug_package %{nil}

Name:           mica
Version:        0.1.0
Release:        1%{?dist}
Summary:        Run programs from other operating systems through the right compatibility backend
License:        BSD-3-Clause
URL:            https://github.com/SwashX-dev/%{repo}
Source0:        %{url}/archive/refs/tags/v%{version}.tar.gz#/%{name}-%{version}.tar.gz

BuildRequires:  cargo >= 1.99
BuildRequires:  rust >= 1.99
BuildRequires:  gcc

Recommends:     wine
Suggests:       steam
Suggests:       qemu-kvm
Suggests:       tpm2-tools

%description
MICA identifies what a program needs and runs it through the right backend:
natively, through Proton or Wine for Windows programs on Linux, WSL on
Windows, or CrossOver on macOS. It checks platform security against
publisher-authorized Linux anti-cheat requirements (Easy Anti-Cheat Linux v2)
and never bypasses, patches, or hides from anti-cheat software.

%prep
%autosetup -n %{repo}-%{version}

%build
RUSTUP_TOOLCHAIN=stable cargo build --locked --release -p mica-cli

%check
RUSTUP_TOOLCHAIN=stable cargo test --locked --release -p mica-inspect -p mica-security -p mica-compat -p mica-engine

%install
install -Dm755 target/release/mica %{buildroot}%{_bindir}/mica

%files
%license LICENSE
%doc README.md docs/ANTI-CHEAT.md
%{_bindir}/mica

%changelog
* Sun Oct 25 2026 MICA contributors - 0.1.0-1
- First package of the mica command.
