# SheetShift codebase guide

SheetShift separates interface concerns from document conversion. The desktop application and CLI call one Rust core so they apply the same supported-format checks, terms requirement, output behavior, and persistence rules.

## Repository map

| Path | Responsibility |
| --- | --- |
| `src/` | React and TypeScript desktop interface, view state, styling, and Tauri IPC calls |
| `src-tauri/` | Tauri application manifest, window configuration, capabilities, Rust command adapters, and native output opening |
| `crates/core/src/lib.rs` | Public request/result/settings types, supported formats, data directory, and terms version |
| `crates/core/src/conversion.rs` | Input parsing, worksheet construction, conversion limits, and XLSX output |
| `crates/core/src/store.rs` | SQLite settings/history, terms gating, failure recording, and retention |
| `crates/cli/src/main.rs` | Clap entry point, interactive menu flow, interrupt handling, and process exit status |
| `crates/cli/src/ui.rs` | Terminal capability detection, shared theme helpers, tables, and JSON/plain print helpers |
| `crates/cli/src/prompts.rs` | Main menu, confirmations, and the fuzzy multi-select file picker |
| `crates/cli/src/progress.rs` | Spinner and batch progress bars for interactive conversions |
| `crates/cli/src/commands/` | Convert, history, settings, terms, and formats subcommand implementations |
| `crates/cli/tests/workflow.rs` | Executable-level CLI tests using temporary profiles and files |
| `docs/TERMS.md` | Shared human-readable terms source |
| `docs/GAPS.md` | Documented limitations and prioritized improvements |

The root Cargo workspace includes `sheetshift-core`, `sheetshift-cli`, and `sheetshift-desktop`. Core and CLI are the default members so routine Rust work does not require desktop system libraries. The frontend is a separate npm/Vite project in the same repository.

## Conversion flow

1. The interface collects input paths and a destination. The CLI processes files sequentially and continues after individual failures.
2. `Store::convert(&ConversionRequest)` loads preferences and requires `accepted_terms_version` to match `TERMS_VERSION`.
3. The conversion engine validates the local input and selects its parser by extension. It extracts document text or structured data into workbook rows.
4. The writer creates an XLSX file in the destination without overwriting an existing file. It returns row counts, sheet counts, and warnings describing relevant losses.
5. The store creates a `HistoryEntry` with status `success` or `failed`. When enabled, it records that entry and trims history to the latest 1,000 attempts.
6. The interface shows the result and warnings. The CLI also reflects batch failure in its exit status.

The core catches unwind panics from third-party parsers and turns them into conversion errors. That protects normal control flow; it does not provide process isolation, prevent allocation failures, or impose a timeout. Terms and database errors return `Err`. Ordinary parser/file failures return `Ok(HistoryEntry)` with `status == "failed"`, allowing batch callers to continue.

If workbook creation succeeds but saving history fails, the error includes the created workbook path. Callers should not interpret that error as proof that no file was created.

## Shared public types

`ConversionRequest` contains `input_path` and `output_directory`, both strings. Interfaces resolve user-selected paths; the core performs file validation and conversion. Rust callers can use relative paths, resolved by their process working directory. The CLI converts paths to absolute Unicode strings before calling the core.

The desktop IPC accepts a nullable destination through `DesktopConversionRequest`. A null destination resolves to the source file's parent folder before calling the core. The CLI instead defaults to `./converted` when no destination preference is saved.

`Settings` contains:

- `output_directory: Option<String>` — a default destination preference.
- `remember_history: bool` — whether to retain future conversion attempts.
- `accepted_terms_version: Option<String>` — explicit acceptance of the current terms.

Only `Store::accept_terms()` updates terms acceptance. Generic `save_settings()` preserves the stored acceptance value so an ordinary preferences update cannot grant consent.

`HistoryEntry` contains an ID, input name/path, optional output path, format, status, UTC RFC 3339 timestamp, row/sheet counts, duration, warnings, and an optional error. The types derive Serde serialization so the desktop IPC and CLI JSON expose the same field names.

The frontend has no separate conversion implementation. Browser preview cannot access the native Rust engine and must communicate that limitation honestly.

## Persistence

`Store` owns a `Mutex<rusqlite::Connection>`. Parsing takes place outside the database mutex. SQLite WAL mode and a five-second busy timeout allow ordinary desktop/CLI access to the same database. They do not make whole conversion operations transactional.

The initial database schema has two tables:

- `preferences`: one row (`id = 1`) containing serialized settings JSON.
- `conversion_history`: monotonically increasing sequence, unique history ID, and serialized entry JSON.

`PRAGMA user_version` identifies schema version 1. Settings use Serde defaults so new fields can have sensible defaults when reading older JSON. Future incompatible changes need explicit, tested migrations; do not silently rewrite the version number as a substitute for migration logic.

History is ordered newest first. Both retention and reads are capped at 1,000 records. `clear_history()` removes metadata and requests a WAL checkpoint; it does not delete documents or settings. Disabling history leaves prior entries intact. The store checks the history preference again after parsing, reducing the chance that a conversion records metadata after the user disables recording during the operation.

The application deliberately does not copy source documents into its database or maintain a background file watcher. A history path may point to a file that was later moved or deleted.

## Desktop boundary and appearance

The Tauri adapter exposes narrowly scoped operations for settings, history, terms, formats, conversions, and opening completed workbooks. Native output opening is limited to `.xlsx` files recognized from successful history or the current session. Native file/folder selection uses the Tauri dialog plugin. Keep capability grants and the content security policy narrow when extending the application.

The frontend follows the operating system's `prefers-color-scheme` preference. There is no persisted light/dark override. Change both palette variants together, check focus visibility and contrast in each, and verify that an operating-system theme change updates an already-open window.

Keep parsing off the UI thread when adding commands. A future cancellation feature must reach the conversion worker; disabling a button or ignoring a promise is not cancellation.

## Format adapters

The core uses `calamine` for spreadsheet input, `pdf-extract` for PDF text, XML/ZIP parsing for DOCX, `csv` for delimited data, `serde_json` for JSON, and `rust_xlsxwriter` for XLSX output. Format support means the documented data extraction is supported, not every feature of a format.

When adding a format:

1. Define the row/column representation and unsupported features before writing the parser.
2. Add the extension to `supported_formats()` and the desktop picker, plus a useful unsupported-file error where relevant.
3. Validate dimensions and allocation limits before creating large in-memory structures.
4. Write source strings as strings. Never interpret a cell beginning with `=`, `+`, `-`, or `@` as a formula merely because it came from a file.
5. Include successful, empty, malformed, and limit-exceeding fixtures. Verify generated workbooks with a separate reader where practical.
6. Update the README conversion matrix, warnings, and gaps document. Increment the terms version only if the terms themselves change.

## Security and privacy boundaries

The Rust process reads the files supplied by the user and writes to the selected destination with that user's operating-system permissions. The document parsers operate in-process. Tauri's webview permissions do not sandbox the Rust parser or restrict it to a disposable process.

Source content should never be executed as macros, scripts, shell commands, or spreadsheet formulas. Outputs are data-only workbooks. Filesystem paths are data; use structured native APIs for opening files instead of concatenating shell command strings.

Database contents include potentially sensitive paths and errors. Neither the metadata database nor generated workbooks are encrypted by the application. Do not add telemetry, remote uploads, background synchronization, or source-content logging without an explicit product and privacy decision.

## Verification and maintenance

Use the commands in the root README for formatting, core/CLI tests, linting, TypeScript/production builds, and the native compile check. Native desktop testing should include multi-file selection, output location changes, individual failures, accepting terms, clearing history, OS theme changes, keyboard navigation, and reopening the application to verify persistence.

Dependency updates should run representative fixture conversions and re-open the generated XLSX files, since a successful build alone cannot establish extraction fidelity. Lock dependencies for repeatable releases, audit their licenses, and publish signed platform artifacts only after testing those exact artifacts.
