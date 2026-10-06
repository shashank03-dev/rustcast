# Security policy

## Supported versions

Security fixes go into the latest release. Please update before reporting.

## Reporting a vulnerability

Please **don't open a public issue** for security problems. Instead, report it
privately through GitHub:
[Report a vulnerability](https://github.com/shashank03-dev/rustcast/security/advisories/new).

Include what you found, how to reproduce it, and what an attacker could do with it.
You'll get a reply within a week, and credit in the release notes if you'd like.

## Scope

RustCast runs with your user's permissions and reads your screen, clipboard and files
when you ask it to. Reports about any of these are in scope, for example:

- clipboard history or screenshots ending up somewhere they shouldn't
- a `rustcast://` link that triggers an action without your consent
- unexpected network access
