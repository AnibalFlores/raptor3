# Security policy

[Español](SECURITY.es.md) · **English**

## Supported versions

Only the **latest published version** in [Releases](https://github.com/AnibalFlores/raptor3/releases) receives security fixes. If you use an older version, update before reporting.

## Reporting a vulnerability

**Do not open a public issue** for security problems. Report them privately by one of these means:

1. **Preferred:** the [**Report a vulnerability**](https://github.com/AnibalFlores/raptor3/security/advisories/new) button in the *Security* tab of the repository (private vulnerability reporting).
2. By email to **jxlfybcrk@mozmail.com**, with the subject `Raptor 3 security`.

Please include:

- The affected version and operating system.
- A description of the problem and its impact.
- The steps to reproduce it, or a proof of concept.
- If you have one, a suggested fix.

## What to expect

Raptor 3 is maintained by one person in their spare time and has no formal support, so response times are on a best-effort basis:

- Acknowledgement of the report: usually within **7 days**.
- After evaluating it, you will be told whether it is accepted as a vulnerability and, if so, the plan for the fix.
- Once the fix is published, the issue is disclosed in a security advisory; you will be credited unless you prefer to remain anonymous.

Please give a reasonable time to publish the fix before disclosing the problem publicly.

## Scope and design decisions

These behaviors are intentional and are **not** considered vulnerabilities on their own:

- **The MCP server has no password.** It listens only on `127.0.0.1` (port `8765`, or the one in `RAPTOR_MCP_PUERTO`) and rejects requests with an `Origin` header from web pages. Anyone who can run programs on the same computer can control the player. Do not expose it to the network or the internet (for example through a tunnel or a port forward).
- **The Windows executable is not digitally signed**, so SmartScreen may warn about it. Download it only from this repository's Releases.
- **External services:** the program queries Open-Meteo (weather) and, only when the user asks for it, MusicBrainz (metadata), without sending personal data.

These **are** of interest, for example:

- Any way to reach the MCP server from another computer or from a web page.
- Reading or writing files outside what the user chose, through the MCP server or through crafted playlists, catalog files or audio files.
- Crashes or code execution caused by malicious audio files or data files.
- Secrets or personal data that end up in the repository, the logs or the published packages.

## Secrets in the repository

The project never stores API keys in the code: the ElevenLabs key and voice used by `herramientas/generar_locuciones.py` are read only from the environment. If you find a secret in the repository or in its history, report it privately by the means above.
