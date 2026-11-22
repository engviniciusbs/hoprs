# Security policy

hoprs handles passwords, private keys and connections to your servers, so security reports
are taken seriously.

## Reporting a vulnerability

Please report it **privately** through
[GitHub's private vulnerability reporting](https://github.com/engviniciusbs/hoprs/security/advisories/new),
not in a public issue, discussion or pull request.

Include what you can: the hoprs version and OS, what an attacker could do, and the steps or
a proof of concept. You will get an answer within a week. Once a fix is released, the
advisory is published and you are credited unless you prefer not to be.

## Supported versions

Only the latest release gets security fixes. Updates are published on the
[releases page](https://github.com/engviniciusbs/hoprs/releases) and at https://hoprs.online.

## Scope

In scope: anything in this repository, for example credential handling (keyring, encrypted
backups), host key and TLS certificate checks, the transfer queue, remote editing, and the
release packages.

Out of scope: vulnerabilities in the servers you connect to, and the known, documented
choices (plain FTP is allowed and labelled unencrypted; the macOS and Windows builds are
not code-signed yet). The test keys and certificates under `docker/fixtures/` and
`crates/*/tests/fixtures/` exist only for the test servers and protect nothing.

## The channel for agents

With Settings › Agents (MCP) on, hoprs listens on a local socket (`agent.sock`; a named pipe
on Windows) that admits the same user only. Any program running as that user can connect and
use the sites whose level allows it, which is the bar the app accepts: such a program can
already read `sites.toml` and ask the keyring. That is why the switch is off by default and
why each site starts at "No access". Nothing listens on the network.
