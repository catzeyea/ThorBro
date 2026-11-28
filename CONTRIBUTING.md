# Contributing to ThorBro

Issues and pull requests are welcome. Help is most useful with:

- Testing on macOS and on different Thor firmware versions.
- Detection for more systems and emulators.

When you report a bug, include your OS, the ThorBro version and what exactly happened.

## Licenses

Code you contribute is licensed under the [MIT License](LICENSE).
The mascot, logo, icons and design files are not MIT. See
[ui/assets/ASSETS-LICENSE.md](ui/assets/ASSETS-LICENSE.md). Don't add new artwork of the mascot
without asking first.

## Build and test

See [Building from source](README.md#building-from-source) in the README for the build.

```bash
cargo test                       # Rust tests
cargo clippy                     # lints
node tests/ui/run.mjs            # UI tests, needs Node.js >= 22 and Firefox
```

More about the UI tests: [tests/ui/README.md](tests/ui/README.md).
Architecture overview: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Releasing a new version

1. Bump the version in `Cargo.toml` and `tauri.conf.json` and commit.
2. Tag and push the tag:

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

3. GitHub Actions builds Windows, Linux and macOS and creates a **draft release**.
   The macOS job may fail without blocking the others.
4. Check the draft on the Releases page and publish it.
