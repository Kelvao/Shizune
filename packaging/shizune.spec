%define _build_id_links none
%global __os_install_post %{nil}
%define __strip /bin/true

Name:           shizune
Version:        %{?pkg_version}%{!?pkg_version:1.0.0}
Release:        1%{?dist}
Summary:        High-performance AI noise suppression virtual microphone for Linux
License:        MIT OR Apache-2.0
URL:            https://github.com/Kelvao/Shizune

%description
High-performance AI noise suppression virtual microphone for Linux.
Real-time deep filter noise reduction with Slint control panel and PipeWire support.

%install
mkdir -p %{buildroot}%{_bindir}
mkdir -p %{buildroot}%{_datadir}/applications
mkdir -p %{buildroot}%{_datadir}/icons/hicolor/scalable/apps

install -m 755 %{_sourcedir}/shizune %{buildroot}%{_bindir}/shizune
install -m 644 %{_sourcedir}/assets/shizune.desktop %{buildroot}%{_datadir}/applications/shizune.desktop
install -m 644 %{_sourcedir}/assets/shizune.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/shizune.svg

%files
%{_bindir}/shizune
%{_datadir}/applications/shizune.desktop
%{_datadir}/icons/hicolor/scalable/apps/shizune.svg

%changelog
* Mon Oct 12 2026 Developer <dev@localhost> - 1.0.0-1
- Initial release

