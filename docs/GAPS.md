# Known gaps and improvement plan

This document distinguishes implemented safeguards from remaining work. The current product is a local converter for document text and spreadsheet data. Supporting “any file” requires separate parsers, fidelity decisions, and tests; a file-extension picker alone cannot provide that capability.

## Gaps addressed in this implementation

| Gap | Current behavior |
| --- | --- |
| Desktop/CLI behavior drifting apart | Both call `sheetshift-core` and share serialized types and a default SQLite profile. |
| Lost conversion context after restarting | Persistent success/failure history contains paths, timestamps, counts, warnings, and errors. |
| Unlimited saved metadata | The latest 1,000 attempts are retained; future history can be disabled and existing history cleared. |
| Silent batch failures | Every input has a result; CLI batches continue and exit nonzero if any input fails. |
| Accidental removal while clearing history | Only metadata is removed; CLI clearing requires `--yes`. |
| Overwriting an existing workbook | The writer selects a unique output name. |
| Terms present only as a UI checkbox | The Rust core checks versioned acceptance for every conversion. |
| Theme disconnected from user preference | Appearance follows the operating-system theme. |
| Browser preview implying conversion support | Preview shows its native-runtime limitation and does not fabricate saved history. |

## Prioritized improvements

### 1. Harden parsing for untrusted and large documents

Document libraries run in the application process. Existing input/dimension checks are useful, but a parser can still spend excessive CPU or memory before returning. Add a worker-process boundary with enforceable memory/time budgets, cancellation, and a clear killed-worker result. Exercise parsers with fuzzing and a corpus of malformed ZIP, PDF, XML, and spreadsheet files. Measure actual peak memory as well as elapsed time.

Acceptance criterion: the application remains responsive and can recover after a deliberately expensive document; no partial workbook is presented as a successful result.

### 2. Improve text extraction fidelity with representative fixtures

PDF text order is determined by the source's text layer. Multiple columns, unusual encodings, rotated text, and positioned glyphs may extract poorly. DOCX body paragraphs and tables do not imply full support for footnotes, headers, text boxes, comments, or tracked changes. Define a fixture corpus with expected content and document the behavior for each feature.

Acceptance criterion: regression tests detect missing text and ordering changes across dependency updates, and the UI reports known omissions in a useful way.

### 3. Add OCR as an explicit optional feature

Image-only PDFs currently cannot become useful text rows. An OCR backend would add language selection, image preprocessing, executable/model distribution, and confidence reporting. Choose a local engine and keep the added disk/CPU cost visible. Do not silently upload scans to a remote service.

Acceptance criterion: users can distinguish extracted text from OCR text, review low-confidence results, and use the application without the optional OCR dependency.

### 4. Offer preview and mapping before writing

The current workflow applies a defined representation for each format. Add a small row preview, delimiter/encoding selection for text, and optional column mapping for JSON. A separate table-extraction mode could later interpret structured PDFs without changing the predictable text-to-rows default.

Acceptance criterion: the preview matches the eventual output and explicit choices are included in the conversion history.

### 5. Add progress and true cancellation

Batch-level progress and busy states do not measure parser progress or make a running parser cancellable. Add per-file progress where libraries support it and cancellation that interrupts the worker safely. Limit concurrency to avoid exhausting memory on a large queue.

Acceptance criterion: cancel leaves no completed-looking partial file and identifies which files succeeded, failed, or were cancelled.

### 6. Extend compatibility deliberately

Legacy `.doc`, `.rtf`, and password-protected files need additional parsers or a clearly managed external converter. Spreadsheet formulas currently become cached values where available; macros, charts, merged-cell semantics, and styling are not retained. Add targeted support only after defining fidelity and security expectations.

Acceptance criterion: each new advertised format has success, failure, and fidelity tests, and missing optional dependencies have actionable errors.

### 7. Strengthen persistence and deletion controls

The first schema is small and JSON-backed. Add tested schema migrations before incompatible changes, recovery/export options for a damaged database, and a configurable retention limit. Concurrent settings writes can overwrite one another because updates read and replace the settings object; use transactional field updates if multi-process configuration editing becomes common. Secure deletion cannot promise removal from filesystem snapshots or external backups.

Acceptance criterion: upgrades preserve existing settings/history, rollback failures are recoverable, and desktop/CLI writes cannot silently lose unrelated preferences.

### 8. Complete distribution and accessibility validation

Run native smoke tests on Windows, macOS, and Linux; verify installer/uninstaller behavior and output opening on each. Add signed release builds, dependency/license inventory, and a documented update policy. Audit keyboard and screen-reader use, focus management in dialogs, reduced motion, high contrast, scaling, and both system themes. Review application terms for the actual distributor and jurisdiction before commercial distribution.

Acceptance criterion: publish only artifacts tested on their target platform, with reproducible build instructions and the correct legal notices.

## Deliberate product boundaries

Cloud accounts, server conversion, analytics, source-file archiving, and exact document-layout reproduction are not implemented. Their absence preserves a straightforward local workflow. Add them only when a concrete user requirement justifies the new complexity, data handling, and maintenance cost.
