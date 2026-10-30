# DriveLedger app

Tauri v2 + React + TypeScript + Tailwind v4. The Rust side (`src-tauri`) is a thin layer of
commands over `driveledger-core`.

```bash
npm install
npm run tauri dev     # development
npm run tauri build   # .app and .dmg in target/release/bundle
```

Fonts (Space Grotesk, JetBrains Mono) are bundled in `src/assets/fonts` under the SIL OFL.
The app icon source is `src-tauri/icons/source/icon.svg` (`npx tauri icon <svg>` regenerates the set).
