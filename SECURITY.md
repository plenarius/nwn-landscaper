# Security

Report a security problem privately, through GitHub: on this repository's **Security** tab, choose **Report a vulnerability**
(a private security advisory). Don't open a public issue for it. Say what an attacker can do, and how to reproduce it.

This repository is the engine: the HTTP server, the files it reads and writes, and signing in on a team server. A problem in the
page that runs in the browser belongs to [nwn-landscaper-web](https://github.com/plenarius/nwn-landscaper-web); report it there
the same way. If you aren't sure which one it is, report it here.

[docs/TEAM-SERVER.md](docs/TEAM-SERVER.md) says what a team server trusts, and what the proxy in front of it must do.
