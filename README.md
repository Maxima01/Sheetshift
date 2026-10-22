# SheetShift

**Convert documents and structured files into clean Excel workbooks — entirely on your own machine.**

SheetShift is a local document-to-Excel converter with a Tauri desktop app and a Rust command-line tool, both driven by the same conversion engine. Word paragraphs, reconstructed PDF tables, spreadsheet values, and plain text data become useful `.xlsx` rows — with no uploads, no cloud processing, and no hidden steps.

- **100% Local & Privacy-First** — Files never leave your device. Conversion runs in a Rust core with no server component.
- **Desktop GUI + CLI** — One-click batch conversion in a native app, or scriptable automation from any terminal.
- **Batch Processing** — Convert many files at once, with an individual result and warning report for every file.
- **Editable Previews** — Review every supported input before export, including worksheet names and cell corrections.
- **PDF Review** — Select page ranges or reconstruct tables from positioned text and cell borders. Paragraphs around detected tables remain as rows; other text is grouped into visual rows.
- **Format Protection** — Optionally retain the original unchanged `.xlsx` workbook package, including merged cells, styling, formulas, links, and embedded content.

---

## Key Capabilities & Supported Formats

SheetShift turns document content into spreadsheet rows, reconstructs ruled PDF tables from their local text layer and cell geometry, and normalizes existing data into `.xlsx` workbooks.

| Input Format | Excel Representation | Key Limitations |
| --- | --- | --- |
| `.docx` | Document paragraphs and tables extracted into worksheets | Body text and table data only; no page layout, images, or Word styling |
| `.pdf` | Ruled tables reconstructed into rows and columns with surrounding text rows; otherwise positioned text is grouped into visual rows | Text-based PDFs only; borderless tables use spacing heuristics, reading order needs review, and image-only pages still need OCR |
| `.xlsx` | Worksheet values, or the original unchanged workbook with Format Protection | Default conversion uses cached values and drops formatting. Format Protection keeps merges, styles, formulas, links, and embedded content but cannot be combined with cell edits or renamed sheets |
| `.xls`, `.xlsb`, `.xlsm`, `.ods` | Worksheet values | Cached values where available; macros, charts, styles, merges, and workbook behavior are not retained |
| `.csv`, `.tsv` | Delimited rows and columns | UTF-8 text; CSV uses comma delimiters and TSV uses tabs |
| `.txt` | Text lines as rows | UTF-8 or UTF-16 with a byte-order mark; no automatic table detection |
| `.json` | Supported JSON data converted into rows and columns | Nested data is represented as JSON text; not an arbitrary schema transformation engine |

**Output**: Clean `.xlsx` workbooks — one workbook per input file, with unique names when needed. Existing files are never overwritten.

> **Explicit scope exclusions**
> Scanned PDFs (no OCR), exact visual layout preservation, legacy `.doc`/`.rtf` files, presentations, and password-protected documents are outside this release's supported scope. Review results and retain your originals. See [known gaps and improvements](docs/GAPS.md).

---

## Quick Start Guide

### Desktop App

1. Launch SheetShift.
2. Add the files you want to convert.
3. Review the destination folder (with no saved destination, workbooks are written beside each source file).
4. Choose **Preview** to inspect the worksheets and apply any corrections.
5. Choose **Convert to Excel**.

The first conversion asks you to review and accept the terms. Settings and the latest 1,000 history entries refresh when you return to the window, including changes made through the CLI. The interface follows your operating system's light or dark appearance, updating live if the system theme changes while the app is open.

### Preview, PDF Review & Format Protection

In the desktop app, add files and choose **Preview** to inspect or correct the eventual worksheet cells. PDFs default to **Find tables**, which groups positioned text using visible cell borders and joins matching page-spanning table sections. If no table is found, the preview shows visual text rows. Choose **Select pages** with a range such as `1-3, 7` to limit extraction; it uses the same table reconstruction as **Find tables**. Wrapped table text stays in one cell, and the preview supports multiline corrections. For an unchanged `.xlsx`, turn on **Format Protection** in the review to retain the original merges, styles, formulas, links, and embedded content. Turn it off to apply cell corrections or rename sheets. SheetShift verifies that the source has not changed between preview and export.

PDF border detection joins short, nearly aligned cell-edge segments before finding the table's full extent. This handles office PDFs that draw each cell border as a separate filled rectangle. Character bounds use independent horizontal and vertical transforms, exact overprints are deduplicated, and a character belongs to only one neighboring cell. Surrounding prose is grouped into paragraphs; table sections separated by an omitted page or different column positions use separate worksheets. These changes follow the geometry-based approach described in [pdfplumber's table extraction documentation](https://github.com/jsvine/pdfplumber#extracting-tables). They improve content structure, while PDF styling, merged cells, scans, and several unrelated tables on one page remain subject to the supported-format limits above.

### CLI Tool

Install the CLI with Cargo (no Node.js or WebView required), then convert in one line:

```sh
cargo install --path crates/cli
sheetshift convert notes.docx report.pdf accounts.csv --output ./workbooks
```

---

## CLI User Guide & Scripting

On an interactive terminal, a bare `sheetshift` opens a main menu (convert, history, settings, terms, formats), and `sheetshift convert` with no file arguments opens a fuzzy multi-select file picker. Interactive batches show animated progress bars and finish with a result table.

When the command is piped, run in CI, or given `--json`, prompts and progress are skipped — scripts get clean plain-text and JSON output. Add `--verbose` for extra diagnostics on stderr. Press Ctrl-C to cancel.

### Batch conversions

```sh
sheetshift convert notes.docx --output ./workbooks
sheetshift convert report.pdf notes.txt accounts.csv --output ./workbooks
sheetshift convert report.pdf notes.docx --output ./workbooks --json
sheetshift convert formatted.xlsx --output ./workbooks --preserve-formatting
```

> Quote paths containing spaces. The output folder comes from `--output`, the saved output preference, or `./converted`, in that order. The CLI creates the folder if necessary after terms have been accepted. Relative paths resolve from the current working directory.

For a noninteractive workflow, read the terms first, then supply acceptance explicitly:

```sh
sheetshift convert notes.docx --output ./workbooks --accept-terms --json
```

The JSON response contains `succeeded`, `failed`, and `results`. A conversion result includes `status` (`success` or `failed`) and any `warnings`. A rejected request can contain only `input_path`, `status`, and `error`. A mixed-success batch continues processing remaining inputs and exits with code `1`.

### Shared preferences

```sh
sheetshift settings view
sheetshift settings output-dir ./workbooks
sheetshift settings output-dir --reset
sheetshift settings history off
sheetshift settings history on
```

Disabling history affects future conversions; it does not erase existing entries.

### History management

```sh
sheetshift history --limit 100 --json
sheetshift history --clear --yes
```

Clearing history requires `--yes` and removes metadata only. Documents, output workbooks, settings, and terms acceptance are left intact.

### Custom state directories

Use a separate profile for scripts or testing:

```sh
sheetshift --data-dir ./local-state terms --accept
sheetshift --data-dir ./local-state convert notes.txt --output ./workbooks
```

### Exit codes

| Code | Meaning |
| --- | --- |
| `0` | All conversions succeeded |
| `1` | Conversion failure or mixed batch (some inputs failed) |
| `2` | Invalid usage (e.g. `history --yes` without `--clear`, or a bare invocation outside a terminal) |
| `130` | Interrupted by Ctrl-C (SIGINT) |

Fatal initialization errors print to stderr and exit with code `1`; JSON stdout is never mixed with prompts or progress messages.

---

## Engine Resource & Safety Limits

The engine rejects work exceeding these hard boundaries:

- **50 MiB** max input file size
- **128 MiB** max expanded ZIP content (10,000 ZIP entries)
- **500,000** max cells per document
- **100,000** max rows per sheet
- **128** max sheets (16,384 columns, 32,767 characters per cell)

> These are application validation limits, not a guarantee of bounded parser memory or execution time.

---

## Local Data, Privacy & Security

SheetShift runs entirely on your machine. Conversions use local Rust libraries and do not upload files to a conversion server. Both the desktop app and CLI share the same Rust conversion engine, settings, terms acceptance, and SQLite history.

**Storage**: Settings, the accepted terms version, and up to 1,000 conversion records live in a single `sheetshift.sqlite3` database in your platform's per-user local data directory (resolved by `directories::ProjectDirs::from("com", "sheetshift", "SheetShift")`):

| Platform | Typical Location |
| --- | --- |
| Windows | `%LOCALAPPDATA%\sheetshift\SheetShift\data` |
| macOS | `~/Library/Application Support/com.sheetshift.SheetShift` |
| Linux | `$XDG_DATA_HOME/sheetshift` (or `~/.local/share/sheetshift`) |

Each history record can include full input/output paths, file name, format, UTC timestamp, duration, row/sheet counts, warnings, and errors. History is metadata, not a copy of your documents. Use `--data-dir` to override the CLI location; the desktop continues using its default profile.

> Database and output files are not encrypted by SheetShift. Access follows your account and filesystem permissions. A cloud-synced source/output/data folder can still be synchronized by other software. The SQLite database may have `-wal` and `-shm` companion files while open. To remove all application state, close both interfaces and remove the database directory — converted files remain wherever you saved them. Clearing history reduces stored metadata but is not a guarantee of forensic erasure from backups or the filesystem.

---

## Developer Setup & Contributing

### Prerequisites

- **Node.js LTS** and npm
- **Rust** stable toolchain
- Platform C++ build tools and WebView (Windows: Visual Studio C++ Build Tools with **Desktop development with C++**, the MSVC Rust toolchain, and Microsoft Edge WebView2; macOS/Linux: respective Tauri system dependencies). Follow the [official Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your platform.

The CLI does not require Node.js or a desktop WebView — only Rust and a working native compiler/linker. Bundled SQLite is compiled as part of the build.

### Desktop App Development

```sh
npm install
npm run tauri dev
```

To build the frontend or create a native application bundle:

```sh
npm run build
npm run tauri build
```

`npm run dev` starts a browser preview of the interface at `http://127.0.0.1:1420`. Native file selection, conversion, persistence, and opening output files require the Tauri application — the preview does not fabricate conversion results or history. Build outputs are placed under `target/release/`; platform installer artifacts, when generated, are under `target/release/bundle/`. Public distribution still needs release signing, packaging verification, and distributor-specific legal notices.

### CLI Development

```sh
cargo build --release -p sheetshift-cli
cargo install --path crates/cli
```

Alternatively, run `target/release/sheetshift` (`sheetshift.exe` on Windows), or use `cargo run -p sheetshift-cli -- ...`.

### Quality Assurance

```sh
cargo fmt --all -- --check
cargo test -p sheetshift-core -p sheetshift-cli
cargo clippy -p sheetshift-core -p sheetshift-cli -- -D warnings
cargo check -p sheetshift-desktop
npm run build
```

For core and CLI tests in PowerShell, `./scripts/test-rust.ps1 -Offline` uses the
workspace's `.tools/env.ps1` when a local toolchain bundle is present, or Cargo on
`PATH` otherwise. The local setup selects the compiler, linker, and bundled Rust
libraries together. Omit `-Offline` when dependencies are not already cached.
See [the conversion module assessment](docs/CONVERSION_REFACTOR.md) for the
proposed parser boundaries and regression coverage before a refactor.

The desktop check requires the platform prerequisites above; core and CLI checks can run without a WebView. CLI integration tests use temporary directories and cover terms gating, clean JSON, mixed batch failures, persistence across processes, history preferences, clearing history without deleting files, and non-interactive usage errors. The frontend build checks TypeScript and production asset generation; it does not replace native UI testing.

---

## Documentation & Project Status

- [**Architecture & Contribution Guide**](docs/ARCHITECTURE.md) — crate responsibilities, conversion flow, storage, and security boundaries.
- [**Gaps & Improvement Plan**](docs/GAPS.md) — supported limits and prioritized next work.
- [**Terms & Conditions**](docs/TERMS.md) — the same versioned terms shown by the app and CLI.

SheetShift is an implementation working name. Before a public release, validate representative real documents, run the native application on each supported operating system, and complete the release items in the [gaps document](docs/GAPS.md). Commercial distribution, jurisdiction-specific legal requirements, and product branding remain undecided.
