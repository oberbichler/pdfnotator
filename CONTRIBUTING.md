# Contributing

Thanks for helping! Bug reports, ideas and pull requests are welcome.

## Setup

```sh
npm install
npm run tauri dev
```

You need Node ≥ 20, stable Rust and the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).
Install [Typst](https://github.com/typst/typst) to run the compile tests (they are skipped otherwise).

## Before you open a pull request

```sh
npm run format   # Biome (TS/CSS/JSON) + rustfmt
npm run check    # typecheck, Biome lint, Vitest, rustfmt --check, Clippy (pedantic, -D warnings), cargo test
```

CI runs the same checks on Linux, macOS and Windows.

## Conventions

- **TypeScript**: strict mode, no `any`, no non-null assertions. Keep logic that can be pure in `src/lib/` and test
  it. UI modules in `src/ui/` render from the store and dispatch actions; they don't own shared state.
- **Rust**: edition 2024, `clippy::pedantic`, no `unsafe`. Keep Tauri-specific code in `lib.rs`, `commands.rs` and
  `events.rs`; everything else must be testable without a running app.
- **Docs**: public items get doc comments explaining _why_, not just _what_.
- **Commits**: imperative mood (“Add area tool”), one logical change per commit.

## UI strings and languages

All UI text lives in `src/i18n/locales/`. `en.ts` is the source of truth.

1. Add the key to `en.ts` (plural messages are objects keyed by `Intl.PluralRules` categories).
2. Add it to **every** other locale — the type checker and `src/i18n/locales.test.ts` fail otherwise.
3. Bind static markup with `data-i18n`, `data-i18n-title`, `data-i18n-placeholder` or `data-i18n-aria-label`;
   use `t("key", { param })` in code.

Native speakers are very welcome to improve translations.

## Screenshots and demo video

Both are produced by the built-in demo mode on `examples/proposal.typ` (macOS, needs a release build, Typst and
ffmpeg):

```sh
npm run tauri build -- --bundles app
./scripts/record-demo.sh      # docs/demo.mp4 + docs/demo.gif — a scripted user and agent (scripts/demo-agent.py)
PDFNOTATOR_DEMO=1 src-tauri/target/release/bundle/macos/PDFnotator.app/Contents/MacOS/pdfnotator \
  "$PWD/examples/proposal.typ" # static scene for docs/screenshot.png
```

Demo mode never changes saved preferences.

## Releasing

1. Bump `version` in `package.json` and `src-tauri/Cargo.toml`, update `CHANGELOG.md`.
2. Commit, tag `vX.Y.Z` and push the tag. The _Release_ workflow builds macOS (arm64 + x64), Windows and Linux
   installers and creates a **draft** GitHub release.
3. Review the draft and publish it.
