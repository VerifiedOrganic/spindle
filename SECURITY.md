# Security Policy

## Reporting Vulnerabilities

Please do not open a public issue for a suspected vulnerability.

Use GitHub private vulnerability reporting if it is enabled for the repository.
If it is not enabled yet, contact the maintainers directly before public
disclosure with:

- a description of the issue
- affected versions or commits, if known
- reproduction steps or proof of concept
- any suggested mitigation

Use a minimal reproduction and avoid including private manuscripts, API keys, or
other sensitive project data.

## Supported Versions

Until Spindle publishes stable releases, security fixes target the current
`main` branch.

## Local Data

Spindle is local-first and stores project data in SQLite under the configured
data directory. Do not share database files, exported Bible payloads, or harness
artifacts unless you have reviewed them for private story content and secrets.
