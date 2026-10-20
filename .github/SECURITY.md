# Security Policy

VectorMagik opens the images you give it and writes files where you choose. It also has a
command line that can run as an MCP server (`--mcp`), so other tools on your PC can call it, and a
version that runs in the browser. A bug in any of those could expose files or let something other
than you drive the program, so we want to hear about it.

## Reporting a vulnerability

If you find a security issue (another program or web page reaching VectorMagik's MCP server or
local functions, a file read or written outside what you chose, or anything else that looks
exploitable) **please report it privately** rather than opening a public issue, so it can be fixed
before it's disclosed:

- open a normal issue **asking us to contact you** (don't include the exploit details there), or
- message the maintainers on [Discord](https://discord.gg/PsWpeNUzhk) and ask for a private
  channel.

Please include:

- what triggers it (the steps, the image or file, the command, or a link to it),
- the **VectorMagik version** and your **operating system**, and
- what you observed (what was reachable, read, written or changed).

We'll acknowledge within a few days and keep you updated through to a fix and release.

## Supported versions

The **latest release** is the supported version, so please update and confirm the issue still
reproduces before reporting.

Thank you for helping keep VectorMagik users safe.
