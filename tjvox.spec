Name:           tjvox
Version:        0.1.0
Release:        1%{?dist}
Summary:        macOS-style voice dictation for Linux

License:        MIT
URL:            https://github.com/tjnome/tjvox
Source0:        tjvox-%{version}.tar.gz

BuildRequires:  rust >= 1.70
BuildRequires:  cargo
BuildRequires:  cmake
BuildRequires:  clang
BuildRequires:  gcc-c++
BuildRequires:  pkgconfig(gtk4)
BuildRequires:  pkgconfig(pipewire-0.3)

Requires:       wl-clipboard
Requires:       wtype
Requires:       pipewire

%description
Voice dictation tool for Linux with Whisper-based transcription,
real-time GUI overlay, and system tray integration.

%prep
%autosetup

%build
cargo build --release

%install
install -Dm755 target/release/tjvox %{buildroot}%{_bindir}/tjvox
install -Dm644 pkg/usr/share/applications/tjvox.desktop %{buildroot}%{_datadir}/applications/tjvox.desktop

%files
%{_bindir}/tjvox
%{_datadir}/applications/tjvox.desktop

%changelog
* Thu Feb 05 2026 TJvox Team <tjvox@example.com> - 0.1.0-1
- Initial package
