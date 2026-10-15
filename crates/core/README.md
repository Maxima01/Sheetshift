# SheetShift conversion core

`sheetshift-core` is the Rust library shared by the Tauri desktop app and CLI.
It reads local files, writes a new `.xlsx` workbook, and stores preferences and
optional conversion metadata in a local SQLite database. It makes no network
requests. No Microsoft Office or LibreOffice installation is required.

## Public API

```rust,no_run
use sheetshift_core::{default_data_dir, ConversionRequest, Store};

fn example() -> anyhow::Result<()> {
    let store = Store::open(default_data_dir()?.join("sheetshift.sqlite3"))?;
    // The caller must present the terms and obtain explicit user acceptance.
    store.accept_terms()?;
    let entry = store.convert(&ConversionRequest {
        input_path: "report.docx".into(),
        output_directory: ".".into(),
    })?;
    if entry.status == "failed" {
        eprintln!("{}", entry.error.unwrap_or_default());
    }
    Ok(())
}
```

`Store::open`, `settings`, `save_settings`, `accept_terms`, `history`,
`clear_history`, and `convert` return `anyhow::Result`. The store supports shared
access through an internal mutex. Parsing does not hold the SQLite lock.
All request, settings, and history types support Serde serialization with
snake_case field names. History timestamps are UTC RFC 3339 strings; consumers
may format them in the user's local timezone.

Conversion outcomes use `status: "success"` or `status: "failed"`. Invalid
input, parser errors, unwritable output directories, and output write failures
become failed history entries. Missing terms acceptance and database errors are
returned as `Err`. A database error can happen after the output was successfully
created; that error includes the created path. Callers should inspect the error
instead of assuming there is no workbook. `save_settings` preserves the stored
terms version; only `accept_terms` records consent.

## Conversion behavior

| Input | Workbook mapping | Deliberate limits |
| --- | --- | --- |
| DOCX | Main-body paragraphs become rows in column A; table rows retain their columns in the same worksheet and body order | No headers, footers, images, formatting, comments, or deleted tracked changes. Merged/nested cells are flattened. Inserted content is included. |
| PDF | A `Document` sheet with `Page`, `Line`, and `Text` columns | Selectable text only; no OCR, table recognition, handwriting, or visual-layout preservation. Extraction order depends on the PDF. Empty pages produce warnings; entirely image-only PDFs fail with an OCR explanation. |
| XLSX, XLS, XLSB, XLSM, ODS | All source worksheets and cell positions copied as values | Cached formula results only. No formulas, macros, charts, styles, merging, external links, or validation. Dates/durations are text or original serial values. Hidden worksheets are included. |
| CSV, TSV | One `Data` sheet; first row is retained unchanged | Comma or tab delimiter according to extension. Ragged rows are accepted. All fields are text to preserve leading zeros and long identifiers. |
| TXT | One text line per row in a `Document` sheet | Blank lines are retained; a final line terminator does not create an additional row. |
| JSON | Object arrays use the sorted union of keys as headers; arrays of arrays become rows; mixed arrays become one cell per item; objects use Key/Value rows | Nested objects/arrays are JSON text. Large numbers with more than 15 digits are text to prevent Excel rounding. Scalar roots and empty objects/arrays are rejected. |

Text inputs accept UTF-8, UTF-8 with a BOM, and UTF-16 LE/BE with a BOM. Unsupported
encodings fail with an instruction to re-save as UTF-8. Input formats are
explicitly enumerated by `supported_formats`; the engine does not promise
arbitrary document conversion. Legacy `.doc`, RTF, PPT/PPTX, and images require
external conversion first.

The library calls `rust_xlsxwriter::write_string` for all source text. Text
beginning with `=`, `+`, `-`, or `@` remains text, and URLs are not converted to
active hyperlinks. Formula and macro execution is not part of conversion.

## Storage and file handling

- `Store::open(path)` creates the database's parent directory if necessary.
- SQLite schema version 1 has a single JSON preferences record and JSON history
  records ordered by an auto-increment sequence. Newer database versions are
  rejected without downgrading their version.
- WAL mode and a five-second busy timeout allow separate desktop and CLI
  processes to use the same database. Settings changes and history insertion use
  immediate transactions to avoid lost consent and history-preference races.
- At most 1,000 conversion attempts are retained. `history(limit)` returns the
  newest first and caps results at 1,000. History stores names, full paths,
  format, time, duration, row/sheet counts, warnings, and errors. It does not store
  source file contents, output contents, or thumbnails.
- Turning history off skips future records; it does not clear existing records.
  `clear_history()` removes history metadata only. Sources, outputs, and
  preferences remain. SQLite secure deletion and WAL checkpointing reduce
  residual database content, but filesystem backups may retain prior data.
- Output folders must already exist. Output files are written completely to a
  temporary file in the destination folder, synced, and persisted with
  `persist_noclobber`. A collision adds a numeric suffix. Existing files and
  original sources are never overwritten; failed temporary writes are cleaned
  up automatically. The destination filesystem must support no-clobber persist.
- Data is local and is **not encrypted by this application**. Use OS account
  permissions and disk encryption when needed. Default data-directory selection
  uses the `directories` crate's per-user local-data location.

System-theme handling belongs to the frontend; the core has no theme override.

## Resource limits and security boundaries

Conversions reject input larger than 50 MiB, archives with more than 10,000
entries or 128 MiB of actual expanded bytes, documents over 500,000 cells,
worksheets over 100,000 rows or 16,384 columns, more than 128 worksheets, and text
cells over Excel's 32,767-character limit. Rejected content is not silently
truncated. ZIP preflight counts actual decompressed bytes and does not extract
entries to disk. Unicode worksheet names and output stems are sanitized and
worksheet names are made unique.

These are application safeguards, **not a hostile-file sandbox**. The core and
third-party parsers process a document in the caller's process. PDF streams and
some spreadsheet structures may allocate memory or consume CPU before a
post-parse row/cell limit can be checked. ZIP validation does not apply to
legacy OLE `.xls` files. PDF extraction has no hard deadline or memory quota.
Rust unwind panics are converted to a failed entry, but out-of-memory aborts
cannot be caught. Output workbooks are assembled in memory before writing.
Separate worker processes, cancellation, resource quotas, richer date handling,
and OCR are tracked as future hardening and fidelity work.

## Code map and verification

- `src/lib.rs`: public request/result/settings types, format list, terms version,
  and default data directory.
- `src/store.rs`: database initialization, preferences, terms acceptance, history
  retention, and orchestration of conversion outcomes.
- `src/conversion.rs`: bounded reading, archive validation, format adapters,
  value normalization, workbook generation, and collision-safe persistence.
- `tests/conversion.rs`: real XLSX readback using Calamine for CSV, JSON, DOCX,
  PDF, UTF-16 TSV, sparse source worksheets, plus terms persistence, private
  history, error records, future-schema rejection, cell limits, and no-overwrite
  behavior.

Run `cargo test -p sheetshift-core` from the workspace root. Tests create their
own temporary fixtures and do not read or modify personal documents.

Parser/API references: [Calamine](https://docs.rs/calamine/0.31.0/calamine/),
[rust_xlsxwriter](https://docs.rs/rust_xlsxwriter/0.92.0/rust_xlsxwriter/),
[pdf-extract](https://docs.rs/pdf-extract/0.10.0/pdf_extract/),
[rusqlite](https://docs.rs/rusqlite/0.32.1/rusqlite/).
