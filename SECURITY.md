# Security Policy

## Reporting a vulnerability

This tool is internal to the UKSF Task Force Alpha unit, but security
matters regardless. If you find a security issue, report it privately
rather than in a public issue.

Do not open a public issue for a vulnerability. Report it to the
maintainers directly, or open a private advisory on GitHub if you have
access.

What to include:

- A description of the issue.
- The affected version or commit.
- Steps to reproduce, if practical.
- Any impact you observed.

## Scope

The tool reads local Steam library records and Workshop pages, and writes
`mod_sources.txt`, `mods.lock`, and generated HTML preset files. It runs
on the user's own machine, not as a network service. The main risk
surfaces are:

- HTML and TOML injection from untrusted mod names or modlist files.
  These are escaped on output.
- Network calls to `steamcommunity.com` and `api.steampowered.com`, and to
  `api.github.com` for the daily update check. IDs are digit-filtered.
- Terminal escape sequences in scraped Workshop text. Control characters
  are stripped before the text is printed.

## Supported versions

Security fixes land on the latest release. Older releases are not
maintained unless the unit says otherwise.
