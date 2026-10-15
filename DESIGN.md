# Sheetshift interface

## Direction

Sheetshift is a calm document workspace. The primary screen carries one main action: bring local files into a queue and turn their useful content into an Excel workbook. The interface uses a blue/slate shell with an emerald workbook cue. Its only illustrative element is a small file-to-workbook icon sequence inside the actual upload target.

This is an Operate surface. Familiar desktop navigation, clear destination controls, per-file results, and keyboard access take priority over visual expression. The native operating-system window frame remains in place.

## Typography

Use locally installed Segoe UI Variable Text, Segoe UI, then platform sans-serif fallbacks. No network font requests. Page titles use 27–34px at weight 650; panel titles 14–17px at weight 620; ordinary content 12–14px. Secondary metadata is compact and uses tabular numerals where appropriate.

## Color tokens

| Role | Light | Dark |
| --- | --- | --- |
| Ground | `#f5f7fb` | `#141b27` |
| Surface | `#ffffff` | `#1d2736` |
| Rail | `#ecf0f7` | `#182230` |
| Text | `#24354c` | `#e1e9f6` |
| Secondary text | `#63728a` | `#a3b1c8` |
| Divider | `#e1e7f0` | `#303d51` |
| Primary action | `#345edb` | `#91b0ff` |
| Workbook/success | `#24785b` | `#94d6b8` |
| Caution | `#875b13` | `#e4be75` |
| Error | `#b13e45` | `#f3a5aa` |

Theme follows `prefers-color-scheme` with no persisted appearance preference or manual override. Native controls use the same color-scheme declaration.

## Layout

- Persistent 232px side rail on wide desktop, narrowing at smaller desktop sizes. A top row of navigation replaces it below 620px.
- A 72px context bar, followed by a focused work area with a maximum width of 1170px.
- The upload target, supported formats, queue, destination, and conversion action form one vertical workflow.
- History uses real records with a scoped search field and status filter. Empty states explain the next useful action.
- Settings use grouped rows and familiar checkbox/toggle controls, with saved state returned by the backend.

## Components and behavior

Major panels have 12px corners and a one-pixel divider, without decorative elevation. Buttons have 7px corners. File-type badges communicate document classes. Selection uses a quiet blue fill; successes use green; failures and extraction warnings retain their semantic colors in both themes.

Only terms acceptance and clearing history interrupt the workflow with modal dialogs. Both use native HTML dialog focus management, Escape support, and visible action labels. Terms content is imported from `docs/TERMS.md` so the desktop and repository share one source.

Multiple files are processed in order. The queue exposes waiting, processing, success, and failure states. Stop finishes the current file before halting the remaining queue. Each completed item can reveal warnings and open its resulting workbook. Failed items can be retried.

## Accessibility and resilience

Visible focus rings, native inputs, navigation semantics, a skip link, status announcements, descriptive icon button labels, and a reduced-motion rule are included. File paths truncate visually but retain a full hover title. Error text wraps rather than expanding the viewport. Inputs and metadata remain readable in both system themes. Browser preview cannot select files, convert, save settings, accept terms, or fabricate history.

## Implementation

`src/styles.css` is the token authority. `src/lib/api.ts` defines the native boundary and shared TypeScript shapes. `src/App.tsx` contains the desktop workflow and its views. There are no external image or font dependencies.
