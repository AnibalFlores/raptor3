# Contributing to Raptor 3

[Español](CONTRIBUTING.es.md) · **English**

Thanks for your interest in Raptor 3. Contributions are welcome through issues and pull requests. The project is maintained by one person in their spare time, so reviews may take a while, and a pull request may be declined if it does not fit the direction of the project.

## How the project is organized

- **The `main` branch is protected.** Changes from contributors go in through pull requests; nobody can push directly, force-push or delete it.
- **Anyone can open issues and pull requests** from their own fork.
- **Collaborators** are people who contribute regularly and are invited by the maintainer. They can create branches in this repository and review pull requests, but their changes also go in through pull requests.
- **Only the maintainer** publishes versions (`v*` tags and releases).
- **Workflows on pull requests from forks** run only after the maintainer approves them.

## Before you start

- **Bugs:** open an issue with the version (*Help › About*), the operating system, the steps to reproduce it and what you expected to happen. If possible, attach the log (`~/.mcp-raptor-player/logs/raptor.log`).
- **Ideas and new features:** open an issue to discuss them **before** writing code, so you don't spend time on something that may not be accepted.
- **Security problems:** do not open a public issue. Follow [SECURITY.md](SECURITY.md).

## Workflow

1. Fork the repository and create a branch from `main` with a descriptive name, for example `fix/crossfade-silence` or `feat/podcast-schedule`.
2. Make your changes, keeping each pull request focused on one topic.
3. Check that everything builds and the tests pass (see below).
4. Open a pull request to `main` explaining **what** changes and **why**. If it changes the user interface, include before and after screenshots.
5. Answer the review comments. The maintainer merges it when it is ready.

### Commit messages

Use a short prefix and describe the change:

- `[FEAT]` new feature
- `[FIX]` bug fix
- `[DOCS]` documentation or manual
- `[REFACTOR]` code change that does not alter behavior
- `[TEST]` tests
- `[BUILD]` build, dependencies or CI

Example: `[FIX] Crossfade no longer cuts the outro of short tracks`.

## Code conventions

- **Language:** the code is written in **Spanish**: names of types, functions and variables, comments and documentation comments. Please keep it that way, with descriptive names.
- **Style:** follow the style of the surrounding code. **Do not reformat code you didn't change** (the project has no `rustfmt` configuration yet, so `cargo fmt` would rewrite whole files).
- **Design:** prefer small, single-purpose functions, and keep the domain logic (`src/dominio`, `src/servicio`) separate from the user interface (`src/interfaz`, `ui/`).
- **Performance:** never block the audio thread or the user interface; long tasks go to their own thread.
- **Dependencies:** avoid adding new ones unless they are clearly needed, and explain why in the pull request. Their licenses must be compatible with GPLv3.

## User interface and translations

- Every user-visible text is written in Spanish and marked for translation: `@tr("...")` in `.slint` files and `tr!("...")` in Rust.
- Add the English translation to `lang/en/LC_MESSAGES/mcp-raptor-player.po`. `python herramientas/traducciones.py --faltantes faltantes.json` lists the missing ones; a test fails if any is missing.
- Check that the windows still fit at the supported resolutions (Full HD at 100 % and 150 %) by generating the screenshots:

```bash
cargo test capturas -- --ignored --test-threads=1
```

- New icons come from [Tabler Icons](https://tabler.io/icons) (MIT). Any other image must state its license in [ASSETS-LICENSES.md](ASSETS-LICENSES.md).

## Tests

```bash
cargo test
```

- All tests must pass on **Windows and Linux** (CI checks both).
- Add tests for every fix or new feature in the domain or service layers.
- Tests must use temporary folders with a unique name and must never touch the user's data. To try the program by hand, set `RAPTOR_CARPETA_DE_DATOS` to a test folder.

## Documentation

If you change behavior that users see, update the manual in both languages (`docs/manual/Raptor_Manual_es.html` and `Raptor_Manual_en.html`) and, if it applies, the READMEs. The README explains how to regenerate the screenshots and the PDFs.

## What not to include

- API keys, tokens or passwords (the ElevenLabs key is read only from the environment).
- Personal data or real paths from your computer.
- Audio files: the HTH voice clips and music beds are not part of the repository.
- Generated files: `target/`, `graphify-out/`.

## License

By contributing, you agree that your contributions are licensed under the [GNU GPLv3 or later](LICENSE), the same license as the project. Images follow what is stated in [ASSETS-LICENSES.md](ASSETS-LICENSES.md).
