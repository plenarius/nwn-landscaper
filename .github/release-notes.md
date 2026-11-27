<!-- The "Getting started" part of each release's notes (.github/workflows/release.yml); no heading of its own. -->
<!-- markdownlint-disable-file MD041 -->

Install the app for your system: it opens the editor in a window of its own, with the engine inside it. Windows:
`nwn-landscaper-*-windows-x86_64-setup.exe` (installs for you alone, no administrator). macOS 11 and newer:
`nwn-landscaper-*-macos-universal.dmg`, then drag NWN Landscaper to Applications. Linux: the `.AppImage`, or the `.deb`
for Debian 12, Ubuntu 22.04 and newer.

The archives hold the program alone, for the command line, for running it on a server, or with your own browser: unpack
one and start the program, and the editor opens in your browser (QUICKSTART.txt in the archive). Each download holds the
engine and the editor of this release (nwn-landscaper-web, same tag).

The macOS app and program are signed with a Developer ID and notarized by Apple: the first start only asks whether to
open a program downloaded from the internet. The Windows installer and programs are signed with Azure Artifact Signing,
so SmartScreen sees a verified publisher. Check a download against SHA256SUMS.

Needs a legally obtained Neverwinter Nights: Enhanced Edition install; ships none of its content. The source is
GPL-3.0-or-later and the binaries include Moonglow (GPL-3.0-only), so they are under GPL-3.0; the editor is MIT. The
complete source of the binaries, every dependency included, is `nwn-landscaper-*-source.tar.gz`. The full history is in
CHANGELOG.md.
