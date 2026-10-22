# Sheetshift

<!-- impeccable:product-schema 1 -->

## Platform

web

The interface is a local desktop application hosted by Tauri. Browser development mode provides an honest visual preview; file conversion requires the desktop runtime.

## Stack

The user requested Tauri, Rust processing, a command-line interface, and codebase documentation. React, TypeScript, and Vite are the implementation choices for the desktop view.

## Product Purpose

Convert supported local documents and structured files into Excel workbooks. People can select multiple files, choose a destination, inspect conversion outcomes, and return to their locally saved history.

## Users

The user confirmed general document text into rows as the primary use case. People prepare documents and tables for spreadsheet work, including repeat users who need to find prior results. No narrower profession or audience was specified.

## Capabilities and Constraints

- Input formats: DOCX, PDF, XLSX, XLS, XLSB, XLSM, ODS, CSV, TSV, TXT, and JSON.
- Output format: XLSX. Extraction is best effort; source structure and supported format determine fidelity.
- Every supported input can be previewed before export; previewed cell values and worksheet names can be corrected.
- PDF review supports one-based page ranges and local table reconstruction from selectable text positions and ruled cell borders, with spacing heuristics as a fallback. Text without a detected table is grouped into visual rows. Page selection avoids extracting unrequested pages.
- Format Protection can retain merges, styling, formulas, links, and embedded content for an unchanged `.xlsx` source. Reviewed cell edits and worksheet renaming require normalized output instead.
- The desktop and CLI share a Rust conversion engine and local persistence.
- History and settings are local. The user can disable future history and clear saved history.
- Terms must be accepted before conversion.
- Theme follows the operating system automatically and updates live.
- Scanned PDFs, unsupported legacy Word files, and exact visual layout reconstruction are outside the initial supported conversion scope.
- No invented sample history, conversion success, or uploaded file claims in browser mode.

## Product Principles

1. Show exactly what will be converted, let the user correct it, and show where it will be saved.
2. Expose individual failures and extraction warnings alongside successful results.
3. Keep routine workflows local and predictable.
4. Make native file selection and keyboard operation first-class.

## Open Decisions

Sheetshift is an implementation working name. Commercial distribution, jurisdiction-specific legal requirements, and product branding remain undecided.
