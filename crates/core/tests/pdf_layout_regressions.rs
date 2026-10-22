use calamine::{open_workbook_auto, Data, Reader};
use sheetshift_core::{
    preview_file, CellEdit, ConversionPreview, PdfReviewMode, PdfReviewOptions, PreviewCell,
    PreviewRequest, ReviewedConversionRequest, Store,
};
use std::{fs, path::Path};

const MULTILINE_TABLE: &str = concat!(
    "0 g 60 620 0.5 130 re f 200 620 0.5 130 re f ",
    "400 620 0.5 130 re f 550 620 0.5 130 re f ",
    "60 620 490 0.5 re f 60 710 490 0.5 re f 60 750 490 0.5 re f ",
    "BT /F1 10 Tf ",
    "1 0 0 1 70 730 Tm (Item) Tj ",
    "1 0 0 1 210 730 Tm (Specification) Tj ",
    "1 0 0 1 410 730 Tm (Compliance) Tj ",
    "1 0 0 1 70 680 Tm (1.10) Tj ",
    "1 0 0 1 210 680 Tm (Height adjustable) Tj ",
    "1 0 0 1 210 660 Tm (Lockable castors) Tj ",
    "1 0 0 1 410 680 Tm (YES) Tj ET"
);

#[test]
fn fragmented_cell_rectangles_form_a_complete_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fragmented-borders.pdf");
    let mut content = String::new();
    // Every cell side is a separate filled rectangle, as in office PDFs.
    // Small gaps and slightly shifted repeated edges must not limit detection
    // to the tallest single row.
    for (bottom, height) in [(630, 40), (671, 30), (702, 48)] {
        for x in [60, 200, 400, 550] {
            content.push_str(&format!("{x} {bottom} 0.5 {height} re f "));
        }
    }
    for y in [630, 671, 702, 750] {
        for (x, width) in [(60, 140), (200, 200), (400, 150)] {
            content.push_str(&format!("{x} {y} {width} 0.5 re f "));
        }
    }
    content.push_str(concat!(
        "BT /F1 10 Tf ",
        "1 0 0 1 70 730 Tm (Item) Tj ",
        "1 0 0 1 210 730 Tm (Specification) Tj ",
        "1 0 0 1 410 730 Tm (Compliance) Tj ",
        "1 0 0 1 70 685 Tm (1.8) Tj ",
        "1 0 0 1 210 685 Tm (Power saving mode) Tj ",
        "1 0 0 1 410 685 Tm (NO BID) Tj ",
        "1 0 0 1 70 655 Tm (1.10) Tj ",
        "1 0 0 1 210 655 Tm (Rechargeable batteries) Tj ",
        "1 0 0 1 210 640 Tm (Charger and manual) Tj ",
        "1 0 0 1 410 655 Tm (YES) Tj ET"
    ));
    fs::write(&path, pdf_with_pages(&[&content])).unwrap();
    for mode in [PdfReviewMode::SelectedPages, PdfReviewMode::DetectTables] {
        let result = preview(&path, mode, None);
        assert_eq!(result.sheets[0].total_rows, 3);
        assert_eq!(
            result.sheets[0].rows[1],
            text_row(&["1.8", "Power saving mode", "NO BID"])
        );
        assert_eq!(
            result.sheets[0].rows[2],
            text_row(&["1.10", "Rechargeable batteries\nCharger and manual", "YES"])
        );
    }
}

#[test]
fn condensed_text_keeps_spaces_and_overprinted_letters_are_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("condensed-text.pdf");
    fs::write(
        &path,
        pdf_with_pages(&[concat!(
            "BT /F1 12 Tf 60 Tz ",
            "1 0 0 1 72 720 Tm (Power saving mode) Tj ",
            "1 0 0 1 72 720 Tm (Power saving mode) Tj ",
            "1 0 0 1 72 690 Tm (Balloon) Tj ET"
        )]),
    )
    .unwrap();
    let result = preview(&path, PdfReviewMode::SelectedPages, None);
    assert_eq!(result.sheets[0].rows[1][2], text("Power saving mode"));
    assert_eq!(result.sheets[0].rows[2][2], text("Balloon"));
}

#[test]
fn manually_selected_pages_reconstruct_multiline_cells_and_exclude_other_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("selected-table.pdf");
    let ignored = MULTILINE_TABLE
        .replace("Height adjustable", "Excluded specification")
        .replace("Lockable castors", "Excluded second line");
    fs::write(
        &path,
        pdf_with_pages(&[
            &ignored,
            MULTILINE_TABLE,
            "BT /F1 12 Tf 72 720 Td (Excluded prose page) Tj ET",
        ]),
    )
    .unwrap();

    let preview = preview(&path, PdfReviewMode::SelectedPages, Some("2"));
    assert_eq!(preview.page_count, Some(3));
    assert_eq!(preview.selected_pages, vec![2]);
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.sheets.len(), 1);
    assert_eq!(preview.sheets[0].total_rows, 2);
    assert_eq!(preview.sheets[0].total_columns, 3);
    assert_eq!(
        preview.sheets[0].rows,
        vec![
            text_row(&["Item", "Specification", "Compliance"]),
            text_row(&["1.10", "Height adjustable\nLockable castors", "YES"]),
        ],
        "A selected page must retain its table cells, not become Page/Line/Text records"
    );
    assert!(!preview_text(&preview).contains("Excluded"));
}

#[test]
fn reviewed_manual_page_exports_match_preview_and_preserve_multiline_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reviewed-table.pdf");
    fs::write(&path, pdf_with_pages(&[MULTILINE_TABLE])).unwrap();
    let options = PdfReviewOptions {
        mode: PdfReviewMode::SelectedPages,
        page_selection: Some("1".into()),
    };
    let preview = preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: options.clone(),
    })
    .unwrap();
    assert_eq!(
        preview.sheets[0].rows[1][1],
        text("Height adjustable\nLockable castors")
    );
    let store = Store::open(dir.path().join("history.sqlite3")).unwrap();
    store.accept_terms().unwrap();

    for edited in [false, true] {
        let corrected = "Height adjustable\nFour lockable castors\nReviewed";
        let entry = store
            .convert_reviewed(&ReviewedConversionRequest {
                input_path: path.to_string_lossy().into_owned(),
                output_directory: dir.path().to_string_lossy().into_owned(),
                source_fingerprint: preview.source_fingerprint.clone(),
                pdf_options: options.clone(),
                edits: if edited {
                    vec![CellEdit {
                        sheet_index: 0,
                        row_index: 1,
                        column_index: 1,
                        value: corrected.into(),
                    }]
                } else {
                    Vec::new()
                },
                sheet_names: vec!["Reviewed specification".into()],
                preserve_formatting: false,
            })
            .unwrap();
        assert_eq!(entry.status, "success", "{:?}", entry.error);
        let mut expected = preview.clone();
        expected.sheets[0].name = "Reviewed specification".into();
        if edited {
            expected.sheets[0].rows[1][1] = text(corrected);
        }
        assert_workbook_matches_preview(entry.output_path.as_deref().unwrap(), &expected);
    }
}

#[test]
fn blank_manual_selection_and_table_detection_keep_the_same_mixed_document() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed-pages.pdf");
    fs::write(
        &path,
        pdf_with_pages(&[
            MULTILINE_TABLE,
            concat!(
                "BT /F1 12 Tf 72 720 Td (Installation notes) Tj ",
                "0 -18 Td (Retain this text-only page.) Tj ET"
            ),
        ]),
    )
    .unwrap();

    let automatic = preview(&path, PdfReviewMode::DetectTables, None);
    for selection in [None, Some(""), Some("   ")] {
        let manual = preview(&path, PdfReviewMode::SelectedPages, selection);
        assert_eq!(manual.selected_pages, vec![1, 2]);
        assert_eq!(manual.detected_tables, 1);
        assert_eq!(manual.sheets, automatic.sheets);
        assert!(preview_text(&manual).contains("Installation notes"));
        assert!(preview_text(&manual).contains("Retain this text-only page."));
    }
}

#[test]
fn nonconsecutive_selected_pages_do_not_join_apparent_row_continuations() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("selection-gap.pdf");
    // Matching columns and a blank item on the later page resemble a wrapped
    // row, but the omitted page makes such a continuation unsafe to infer.
    let first = concat!(
        "60 40 490 140 re S ",
        "200 40 m 200 180 l S 400 40 m 400 180 l S ",
        "60 140 m 550 140 l S ",
        "BT /F1 10 Tf ",
        "1 0 0 1 70 160 Tm (Item) Tj ",
        "1 0 0 1 210 160 Tm (Specification) Tj ",
        "1 0 0 1 410 160 Tm (Compliance) Tj ",
        "1 0 0 1 70 100 Tm (1) Tj ",
        "1 0 0 1 210 100 Tm (First selected fragment) Tj ",
        "1 0 0 1 410 100 Tm (YES) Tj ET"
    );
    let third = MULTILINE_TABLE
        .replace("(1.10) Tj", "() Tj")
        .replace("Height adjustable", "Later selected fragment")
        .replace("Lockable castors", "A separate row")
        .replace("(YES) Tj", "() Tj");
    fs::write(
        &path,
        pdf_with_pages(&[
            first,
            "BT /F1 12 Tf 72 720 Td (Omitted middle page) Tj ET",
            &third,
        ]),
    )
    .unwrap();

    let preview = preview(&path, PdfReviewMode::SelectedPages, Some("1,3"));
    assert_eq!(preview.selected_pages, vec![1, 3]);
    assert_eq!(preview.detected_tables, 2);
    let rows = preview
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.rows)
        .collect::<Vec<_>>();
    let first_row = rows
        .iter()
        .position(|row| row.contains(&text("First selected fragment")))
        .expect("The first page's cell must remain intact");
    let later_row = rows
        .iter()
        .position(|row| row.contains(&text("Later selected fragment\nA separate row")))
        .expect("The later page's cell must remain separate across a selection gap");
    assert_ne!(first_row, later_row);
    assert!(!preview_text(&preview).contains("Omitted middle page"));
}

fn preview(path: &Path, mode: PdfReviewMode, selection: Option<&str>) -> ConversionPreview {
    preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: PdfReviewOptions {
            mode,
            page_selection: selection.map(str::to_owned),
        },
    })
    .unwrap()
}

fn text(value: &str) -> PreviewCell {
    PreviewCell::Text(value.into())
}

fn text_row(values: &[&str]) -> Vec<PreviewCell> {
    values.iter().map(|value| text(value)).collect()
}

fn preview_text(preview: &ConversionPreview) -> String {
    preview
        .sheets
        .iter()
        .flat_map(|sheet| &sheet.rows)
        .flatten()
        .filter_map(|cell| match cell {
            PreviewCell::Text(value) => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_workbook_matches_preview(path: &str, preview: &ConversionPreview) {
    let mut workbook = open_workbook_auto(path).unwrap();
    assert_eq!(
        workbook.sheet_names(),
        preview
            .sheets
            .iter()
            .map(|sheet| sheet.name.clone())
            .collect::<Vec<_>>()
    );
    for sheet in &preview.sheets {
        assert!(!sheet.truncated);
        let range = workbook.worksheet_range(&sheet.name).unwrap();
        assert_eq!(range.height(), sheet.total_rows);
        assert_eq!(range.width(), sheet.total_columns);
        for (row_index, row) in sheet.rows.iter().enumerate() {
            for (column_index, expected) in row.iter().enumerate() {
                let actual = range.get((row_index, column_index)).unwrap_or(&Data::Empty);
                let actual = match actual {
                    Data::String(value) => PreviewCell::Text(value.clone()),
                    Data::Int(value) => PreviewCell::Number(*value as f64),
                    Data::Float(value) => PreviewCell::Number(*value),
                    Data::Bool(value) => PreviewCell::Boolean(*value),
                    Data::Empty => PreviewCell::Empty,
                    unexpected => panic!("Unexpected exported cell: {unexpected:?}"),
                };
                assert_eq!(&actual, expected, "row {row_index}, column {column_index}");
            }
        }
    }
}

/// Small deterministic PDFs keep these regressions independent of user files
/// and preserve the positioned text and vector borders used by the parser.
fn pdf_with_pages(contents: &[&str]) -> Vec<u8> {
    let font_id = 3 + contents.len();
    let kids = (0..contents.len())
        .map(|index| format!("{} 0 R", index + 3))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!(
            "<< /Type /Pages /Kids [{kids}] /Count {} >>",
            contents.len()
        ),
    ];
    for index in 0..contents.len() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font_id} 0 R >> >> /Contents {} 0 R >>",
            font_id + index + 1
        ));
    }
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for content in contents {
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }

    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    let size = objects.len() + 1;
    output.extend(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}
