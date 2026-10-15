# SheetShift

A local document-to-Excel converter with a Tauri desktop interface and a Rust command-line tool. The primary workflow turns general document text into spreadsheet rows: Word paragraphs, PDF text lines, and plain text become useful Excel data. Existing spreadsheets and structured text can also be normalized to `.xlsx`.

Both interfaces use the same Rust conversion engine, settings, terms acceptance, and SQLite history. Source documents stay on your device. This project does not promise conversion of every document format or pixel-perfect reproduction of document layouts.

## What is included

- Batch conversion with an individual result and warnings for each file.
- DOCX, text PDF, XLSX, XLS, XLSB, XLSM, ODS, CSV, TSV, TXT, and JSON inputs; XLSX output.
- Local settings for the default destination and whether future history is recorded.
- Conversion history with source/output paths, timestamps, outcomes, row counts, and errors. The latest 1,000 attempts are retained.
- Versioned terms acceptance enforced by the Rust engine in both desktop and CLI.
- Light and dark appearance that follows the operating system, including changes while the app is open.
- CLI batch JSON output and a nonzero exit code when any conversion fails.

## Supported conversion behavior

| Input | Excel representation | Important limitations |
| --- | --- | --- |
| `.docx` | Document paragraphs and tables extracted into worksheets | Body text and table data; not page layout, images, or Word styling |
| `.pdf` | Extractable text lines in `Page`, `Line`, and `Text` columns | Text-based PDFs only; no OCR or reconstruction of visual tables; reading order depends on the PDF |
| `.xlsx`, `.xls`, `.xlsb`, `.xlsm`, `.ods` | Worksheet values | Cached values where available; formulas, macros, charts, styles, and workbook behavior are not retained |
| `.csv`, `.tsv` | Delimited rows and columns | UTF-8 text; CSV uses comma delimiters and TSV uses tabs |
| `.txt` | Text lines as rows | UTF-8 or UTF-16 with a byte-order mark; not automatic table detection |
| `.json` | Supported JSON data converted into rows and columns | Nested data is represented as JSON text; not an arbitrary schema transformation engine |

Legacy Word `.doc`, `.rtf`, presentations, scanned PDFs, password-protected documents, and exact layout preservation are outside this release's supported scope. Review results and retain originals. See [known gaps and improvements](docs/GAPS.md).

The engine rejects work exceeding 50 MiB per input, 128 MiB expanded ZIP content, 10,000 ZIP entries, 500,000 cells per document, 100,000 rows per sheet, 128 sheets, 16,384 columns, or 32,767 characters per cell. These are application limits, not a guarantee of bounded parser memory or execution time.

## Develop the desktop application

Install a supported Node.js LTS release, npm, and the current stable Rust toolchain. On Windows, install the Visual Studio C++ Build Tools with **Desktop development with C++**, the MSVC Rust toolchain, and Microsoft Edge WebView2. macOS and Linux require their respective Tauri system dependencies. Follow the [official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

From the repository root:

```sh
npm install
npm run tauri dev
```

To build the frontend or create a native application bundle:

```sh
npm run build
npm run tauri build
```

`npm run dev` starts a browser preview of the interface at `http://127.0.0.1:1420`. Native file selection, conversion, persistence, and opening output files require the Tauri application. The preview deliberately does not fabricate conversion results or history.

In the desktop app, add files, review the destination, and choose **Convert to Excel**. With no saved destination, workbooks are written beside each source file. The first conversion asks you to review and accept the terms. Settings and the latest 1,000 history entries refresh when you return to the window, including changes made through the CLI.

Build outputs are placed under `target/release/`; platform installer artifacts, when generated, are under `target/release/bundle/`. Public distribution still needs release signing, packaging verification, and distributor-specific legal notices.

## Build and use the CLI

The CLI does not require Node.js or a desktop WebView. It needs Rust and a working native compiler/linker; bundled SQLite is compiled as part of the build.

```sh
cargo build --release -p sheetshift-cli
cargo install --path crates/cli
sheetshift --help
sheetshift formats
sheetshift terms
sheetshift terms --accept
```

`cargo install --path` installs `sheetshift` in Cargo's binary directory. Alternatively, run `target/release/sheetshift` (`sheetshift.exe` on Windows), or use `cargo run -p sheetshift-cli -- ...`. See [Cargo's install documentation](https://doc.rust-lang.org/cargo/commands/cargo-install.html).

Convert one file or a batch:

```sh
sheetshift convert notes.docx --output ./workbooks
sheetshift convert report.pdf notes.txt accounts.csv --output ./workbooks
sheetshift convert report.pdf notes.docx --output ./workbooks --json
```

Quote paths containing spaces. The output folder comes from `--output`, the saved output preference, or `./converted`, in that order. The CLI creates that folder if necessary after terms have been accepted. Relative paths resolve from the current working directory. Every input creates a separate workbook. Existing files are not overwritten; a unique output name is chosen when needed.

For a noninteractive workflow, read the terms first, then explicitly supply acceptance:

```sh
sheetshift convert notes.docx --output ./workbooks --accept-terms --json
```

The JSON response contains `succeeded`, `failed`, and `results`. A conversion result contains `status` (`success` or `failed`) and any `warnings`. A rejected request can contain only `input_path`, `status`, and `error`. A mixed-success batch continues processing remaining inputs and exits with code `1`. Invalid command syntax exits with code `2`. Fatal initialization errors are printed to stderr and exit with code `1`; JSON stdout is never mixed with progress messages.

Manage shared preferences and history:

```sh
sheetshift settings view
sheetshift settings output-dir ./workbooks
sheetshift settings output-dir --reset
sheetshift settings history off
sheetshift settings history on
sheetshift history --limit 100 --json
sheetshift history --clear --yes
```

Disabling history affects future conversions; it does not erase existing entries. Clearing history requires `--yes` and removes metadata only. It leaves your documents, output workbooks, settings, and terms acceptance intact.

Use a separate profile for scripts or testing:

```sh
sheetshift --data-dir ./local-state terms --accept
sheetshift --data-dir ./local-state convert notes.txt --output ./workbooks
```

## Local data and privacy

The CLI and desktop app share `sheetshift.sqlite3` in the platform's per-user local data directory, resolved by `directories::ProjectDirs::from("com", "sheetshift", "SheetShift")`. On Windows this is normally `%LOCALAPPDATA%\sheetshift\SheetShift\data`. Use `--data-dir` to override the CLI location; the desktop continues using its default profile.

SQLite stores settings, the accepted terms version, and up to 1,000 conversion records. Each record can include the full input and output paths, file name, format, UTC timestamp, duration, row/sheet counts, warnings, and errors. History is metadata, not a copy of the source document. Database and output files are not encrypted by SheetShift. Access follows your account and filesystem permissions.

Conversions use local Rust libraries and do not upload files to a conversion server. A cloud-synced source/output/data folder can still be synchronized by other software. The SQLite database can have `-wal` and `-shm` companion files while open. To remove all application state, close both interfaces and remove the database directory. Converted files remain wherever you saved them. Clearing history reduces stored metadata but is not a guarantee of forensic erasure from backups or the filesystem.

## Check the code

```sh
npm run build
cargo fmt --all -- --check
cargo test -p sheetshift-core -p sheetshift-cli
cargo clippy -p sheetshift-core -p sheetshift-cli -- -D warnings
cargo check -p sheetshift-desktop
```

The desktop check requires the platform prerequisites above. Core and CLI checks can run without a WebView. CLI integration tests use temporary directories and cover terms gating, clean JSON, mixed batch failures, persistence across processes, history preferences, and clearing history without deleting files. The frontend build checks TypeScript and production asset generation; it does not replace native UI testing.

## Codebase documentation

- [Architecture and contribution guide](docs/ARCHITECTURE.md): crate responsibilities, conversion flow, storage, and security boundaries.
- [Gaps and improvement plan](docs/GAPS.md): supported limits and prioritized next work.
- [Terms and conditions](docs/TERMS.md): the same versioned terms shown by the app and CLI.

Before a public release, validate representative real documents, run the native application on each supported operating system, and complete the release items in the gaps document.
