# ClipStack

> A cross-platform clipboard manager · Never lose what you copied again.

ClipStack is a **cross-platform clipboard manager** that lives in your menu bar / system tray and runs quietly in the background. It supports **macOS** and **Windows**. It captures everything you copy — text, links, code, images, and files — in real time, and gives you full-text search, smart categorization, and one-click reuse, so your clipboard becomes **searchable, browsable, and reusable**.

- App identifier: `tech.newxin-clipstack.app`
- Current version: `0.1.0`
- Stack: Tauri 2 + React 18 + TypeScript + Vite + Rust (SQLite persistence)

---

## 1. Key Features

- **Real-time capture (event-driven)**: Listens to system clipboard change events (macOS uses `NSPasteboard.changeCount` diffing, not polling) — low power, low overhead. Captures five content types: plain text, links, code, images, and files.
- **History management**: Reverse-chronological list grouped by day; "pinned" items always stay on top, "favorited" items are independently flagged so important entries never get buried.
- **Trash**: Deleted items go to the trash where they can be restored or permanently cleared. Both images and trashed items support **preview**.
- **Search & filter**: Full-text search (focus with `⌘K` / `Ctrl+K`), category switching (All / Text / Link / Code / Image / File), and time filtering (Today / Yesterday / This week / All).
- **One-click copy**: Click or press Enter to copy an item back to the system clipboard (text / link / code). Copying images and files is disabled for now due to platform API limits.
- **Menu bar / system tray**: Lives in the macOS menu bar or Windows system tray; dropdown shows recent history for one-click paste. The number of items is configurable in Settings (default 30).
- **Global shortcut**: `⌘⇧V` / `Ctrl Shift V` toggles the main window. Closing the main window hides it instead of quitting — the app stays resident in the tray.
- **Appearance themes**: Light / Dark / Follow system (default is "Follow system", powered by Tauri's native theme API).
- **Privacy fence (ignored apps)**: Skip specific apps (e.g. password managers) by typing a name or picking from installed apps; matched apps are never captured.
- **Multilingual UI**: Built-in 简体中文 / 繁體中文 / English / 日本語 / Deutsch / Français, following the system or set manually.
- **Lightweight**: Rust backend with the system's native WebView — **no Node runtime** — so its resident memory footprint is far below comparable Electron-based tools.

---

## 2. Supported Platforms

| Platform | Form | Status |
|---|---|---|
| macOS | Menu bar app (`.dmg` / `.app`) | ✅ Supported |
| Windows | System tray app (`.msi`) | ✅ Supported |
| Linux | Tray app | Optional (planned) |

> On macOS, the first run may require granting ClipStack the **Accessibility** permission in System Settings → Privacy & Security → Accessibility, so clipboard monitoring and the global shortcut work correctly.

---

## 3. Install & Run

### Option A: Download the installer (recommended)

- **macOS**: Download the `.dmg` and drag the app into Applications.
- **Windows**: Download the `.msi` and follow the setup wizard.

> Release builds must be signed and notarized. Unsigned installers may be blocked by Gatekeeper / SmartScreen on other machines. See [`clipstack/docs/clipstack-packaging.md`](clipstack/docs/clipstack-packaging.md).

### Option B: Build from source

Prerequisites: Node ≥ 22, Rust toolchain (rustup + cargo), and Xcode Command Line Tools on macOS.

```bash
git clone <repo-url>
cd clipstack
npm install
npm run tauri build      # produces installers: macOS → .dmg/.app, Windows → .msi
```

For development preview:

```bash
npm run tauri dev        # run the full Tauri app (frontend + native window)
```

---

## 4. Quick Start

1. Launch ClipStack — it resides in the menu bar / tray.
2. Copy as usual (`⌘C` / `Ctrl C`) — ClipStack records it automatically.
3. Press `⌘⇧V` / `Ctrl Shift V` to open the main window, or click the menu bar / tray icon for recent history.
4. Press `⌘K` / `Ctrl K` to search; press Enter or click to copy an item back to the clipboard.
5. Right-click an item to pin, favorite, or delete (deletes go to the trash and can be restored).

---

## 5. Technical Architecture

| Layer | Choice | Notes |
|---|---|---|
| Framework | Tauri 2.x | Lightweight, no bundled browser engine |
| Frontend | React 18 + TypeScript + Vite | Mature ecosystem, type-safe |
| State | Zustand | Single source of truth, lightweight |
| Styling | CSS-variable design tokens | Theme-driven |
| Backend | Rust (edition 2021) | Capture / tray / shortcut / DB |
| Database | SQLite (rusqlite) | Local-first, privacy-friendly |
| Plugins | global-shortcut / tray-icon / dialog / autostart / clipboard-manager / opener / process | Native capabilities |

### Project Structure

```
clipboards/
├── clipstack/                # The ClipStack project
│   ├── src/                  # Frontend (React + TS)
│   │   ├── components/       # Sidebar / HistoryList / DetailPanel / Settings / Trash
│   │   ├── store/            # Zustand state
│   │   ├── lib/              # invoke wrappers, actions, theme, i18n
│   │   └── styles/           # Design tokens & base styles
│   ├── src-tauri/            # Backend (Rust)
│   │   └── src/              # clipboard / db / commands / tray / models ...
│   └── README.md             # Developer reference (build, layout, conventions)
└── docs/                     # Planning & build docs
    ├── clipstack-development-plan.md
    ├── clipstack-build-steps.md
    └── clipstack-packaging.md
```

---

## 6. Data Model (SQLite)

The local database contains four tables: `history`, `trash`, `settings`, and `ignored_apps`.

| Table | Key columns | Notes |
|---|---|---|
| history | content_type / content_text / content_blob / source_app / hash / is_pinned / is_favorite / created_at | Clipboard entries; `hash` used for deduplication |
| trash | same as history + deleted_at | Soft-deleted, restorable |
| settings | key / value | Appearance, capacity, autostart, etc. |
| ignored_apps | name | Skip capture by app name (case-insensitive) |

All data is stored locally with no cloud sync by default — your clipboard stays private.

---

## 7. For Developers

For the full developer reference — environment setup, build commands, coding conventions, data model, and packaging/signing flow — see the in-repo docs:

- [`clipstack/README.md`](clipstack/README.md) — Developer reference (environment, commands, layout, theme, ignored apps, progress)
- [`docs/clipstack-development-plan.md`](docs/clipstack-development-plan.md) — Product positioning, feature plan, architecture, conventions, milestones
- [`docs/clipstack-build-steps.md`](docs/clipstack-build-steps.md) — Step-by-step build log & progress
- [`docs/clipstack-packaging.md`](docs/clipstack-packaging.md) — Local build, macOS signing + notarization, Windows `.msi`, CI

Quality gates: Rust `cargo test` (24/24), `cargo clippy --all-targets` (0 warnings); frontend `npm run build` (tsc + vite) passes.

---

## 8. Known Limitations

- One-click copy for images / files is not yet implemented (buttons disabled) due to platform API limits; planned for a future release.
- Actual signing, notarization, and packaging must be performed on a developer machine or CI; the sandbox cannot run them.
- The macOS menu bar tray icon renders as a template image by default (may appear monochrome) — this is native behavior.

---

## 9. License

See the repository license file.
