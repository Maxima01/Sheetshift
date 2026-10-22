# Conversion module separation

The shared engine still coordinates input validation, preview and review edits,
XLSX output, and persistence. PDF parsing is the first format-specific node,
now in `crates/core/src/conversion/pdf.rs`. Other parser families remain in
`conversion.rs` for later incremental extraction.

## Proposed boundaries

Keep `conversion.rs` as the private coordinator used by the existing public API.
Its child modules can live in `crates/core/src/conversion/`:

| Module | Responsibility |
| --- | --- |
| `model.rs` | Normalized cells, sheets, parsed documents, and document-wide cell accounting |
| `input.rs` | Bounded file reading, supported-extension checks, and ZIP preflight |
| `pdf.rs` | PDF page selection, positioned text, ruled tables, fallback tables, and extraction warnings (extracted) |
| `docx.rs` | DOCX body paragraphs, tables, and document-order traversal |
| `spreadsheet.rs` | XLSX, XLS, XLSB, XLSM, and ODS input through Calamine |
| `text.rs` | Shared text decoding, CSV/TSV records, and plain text rows |
| `json.rs` | JSON row mapping and numeric precision policy |
| `output.rs` | XLSX cell writing, layout, sheet names, unchanged XLSX copying, and atomic output persistence |

Group formats by their existing parsing implementation. XLSX, XLS, XLSB, XLSM,
and ODS already share one parser; CSV and TSV differ by delimiter. Separate files
for every extension would duplicate those implementations. If PDF remains hard
to navigate after extraction, split its geometry and table detection into child
modules in a later change.

## Contracts that make the split useful

Parsers should receive validated bytes and explicit format options, then return a
common parsed document with warnings and metadata. The coordinator should keep
preview truncation, source fingerprint checks, review edits, and final export in
one flow so preview and export continue to use the same parser and options.

Keep document limits and cell accounting shared across every parser and sheet.
Giving each format or sheet its own budget would weaken the existing overall
500,000-cell limit. Parsers should build the common model rather than write XLSX
files themselves. The output module should continue to own formula-safe text
writing, output collision handling, and persistence.

After the mechanical extraction, give sheets an explicit layout kind such as
`Data`, `TextLines`, `PdfTextRows`, or `PdfTable`. The current writer infers PDF
text rows by matching the first row against `Page`, `Line`, and `Text`; editing a
heading can therefore change export formatting. An explicit kind carries that
intent through review without depending on cell contents. Calculate effective
column widths once and use the same widths for writing and row-height estimates.

The PDF node now returns named fields for sheets, page count, included pages, and
table count. In automatic table mode, the page list includes pages contributing
selectable content, even when a page contains only surrounding paragraphs.

These modules isolate code ownership and tests. Parser crashes, excessive memory
use, and timeouts still require a separate worker process to isolate execution.

## Suggested sequence and verification

1. The core and CLI baseline passed with the configured local toolchain.
2. PDF regression cases now cover empty spacing-table fields, paragraphs around
   tables, and a text-only page following a ruled table. Both preview and XLSX
   output are checked.
3. The PDF parser has moved to its own node, preserving the public API and the
   shared 500,000-cell budget for each parsed document.
4. Next, extract DOCX, spreadsheet, text, and JSON parsers one family at a time.
   Keep preview and export on the same parser path for each family.
5. Add explicit sheet layout kinds so the writer does not infer PDF presentation
   from editable cell values. Then test long wrapped cells and narrow columns.

Retain integration tests that reopen generated XLSX files and exercise the CLI.
Add parser-level tests close to each module for extraction edge cases; use output
XML assertions for written widths, wrapping, and row heights. A passing extraction
test alone cannot verify spreadsheet presentation. Keep shared tests for limits,
source-change rejection, reviewed edits, Format Protection, and no-overwrite
persistence. The current tests cover several representative fixtures but do not
cover all three previously identified PDF edge cases.

This stage separates code ownership. A parser crash, allocation failure, or
timeout still needs a worker process for runtime isolation.
