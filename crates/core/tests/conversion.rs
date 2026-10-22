use calamine::{open_workbook_auto, Data, Reader};
use sheetshift_core::{
    preview_file, CellEdit, ConversionRequest, PdfReviewMode, PdfReviewOptions, PreviewCell,
    PreviewRequest, ReviewedConversionRequest, Settings, Store, TERMS_VERSION,
};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
};
use tempfile::TempDir;

fn ready() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("data/history.sqlite3")).unwrap();
    store.accept_terms().unwrap();
    (dir, store)
}

fn request(dir: &Path, filename: &str) -> ConversionRequest {
    ConversionRequest {
        input_path: dir.join(filename).to_string_lossy().into_owned(),
        output_directory: dir.to_string_lossy().into_owned(),
        preserve_formatting: false,
    }
}

fn read_first(path: &str) -> calamine::Range<Data> {
    let mut workbook = open_workbook_auto(path).unwrap();
    let name = workbook.sheet_names()[0].clone();
    workbook.worksheet_range(&name).unwrap()
}

#[test]
fn csv_preserves_identifiers_and_formula_like_text_without_overwriting() {
    let (dir, store) = ready();
    fs::write(
        dir.path().join("data.csv"),
        "name,id,formula\nAlice,00123,=HYPERLINK(\"\"bad\"\")\nBob,00007,+SUM(A1)\n",
    )
    .unwrap();
    let first = store.convert(&request(dir.path(), "data.csv")).unwrap();
    assert_eq!(first.status, "success", "{:?}", first.error);
    assert_eq!(first.rows, 3);
    assert_eq!(first.sheets, 1);
    let output = first.output_path.unwrap();
    let original = fs::read(&output).unwrap();
    let range = read_first(&output);
    assert_eq!(range.get((1, 1)), Some(&Data::String("00123".into())));
    assert!(matches!(range.get((1, 2)), Some(Data::String(s)) if s.starts_with("=HYPERLINK")));
    assert_eq!(range.get((2, 2)), Some(&Data::String("+SUM(A1)".into())));
    let mut zip = zip::ZipArchive::new(Cursor::new(&original)).unwrap();
    let mut sheet_xml = String::new();
    zip.by_name("xl/worksheets/sheet1.xml")
        .unwrap()
        .read_to_string(&mut sheet_xml)
        .unwrap();
    assert!(!sheet_xml.contains("<f>"));
    let second = store.convert(&request(dir.path(), "data.csv")).unwrap();
    assert_eq!(second.status, "success");
    assert_ne!(second.output_path.as_deref(), Some(output.as_str()));
    assert_eq!(fs::read(output).unwrap(), original);
    assert_eq!(store.history(50).unwrap().len(), 2);
}

#[test]
fn terms_require_explicit_acceptance_and_persist_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite3");
    let store = Store::open(&path).unwrap();
    fs::write(
        dir.path().join("note.txt"),
        "A paragraph\nAnother paragraph",
    )
    .unwrap();
    let error = store.convert(&request(dir.path(), "note.txt")).unwrap_err();
    assert!(error.to_string().contains("terms"));
    assert!(store.history(10).unwrap().is_empty());
    store
        .save_settings(&Settings {
            accepted_terms_version: Some(TERMS_VERSION.into()),
            ..Settings::default()
        })
        .unwrap();
    assert_eq!(store.settings().unwrap().accepted_terms_version, None);
    store.accept_terms().unwrap();
    store.save_settings(&Settings::default()).unwrap();
    assert_eq!(
        store.settings().unwrap().accepted_terms_version.as_deref(),
        Some(TERMS_VERSION)
    );
    assert_eq!(
        store
            .convert(&request(dir.path(), "note.txt"))
            .unwrap()
            .status,
        "success"
    );
    drop(store);
    let reopened = Store::open(path).unwrap();
    assert_eq!(reopened.history(10).unwrap().len(), 1);
    assert_eq!(
        reopened
            .settings()
            .unwrap()
            .accepted_terms_version
            .as_deref(),
        Some(TERMS_VERSION)
    );
}

#[test]
fn failures_are_recorded_and_history_can_be_disabled_and_cleared() {
    let (dir, store) = ready();
    let failure = store.convert(&request(dir.path(), "missing.csv")).unwrap();
    assert_eq!(failure.status, "failed");
    assert!(failure.error.unwrap().contains("Could not open"));
    assert_eq!(store.history(10).unwrap().len(), 1);
    let mut settings = store.settings().unwrap();
    settings.remember_history = false;
    settings.output_directory = Some(dir.path().to_string_lossy().into_owned());
    store.save_settings(&settings).unwrap();
    fs::write(dir.path().join("note.txt"), "Private note").unwrap();
    let entry = store.convert(&request(dir.path(), "note.txt")).unwrap();
    assert_eq!(entry.status, "success");
    assert_eq!(store.history(10).unwrap().len(), 1);
    store.clear_history().unwrap();
    assert!(store.history(10).unwrap().is_empty());
    assert!(Path::new(entry.output_path.as_ref().unwrap()).exists());
    assert!(dir.path().join("note.txt").exists());
}

#[test]
fn docx_keeps_paragraph_and_table_order() {
    let (dir, store) = ready();
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
      <w:p><w:r><w:t>Introduction</w:t></w:r></w:p>
      <w:tbl><w:tr><w:tc><w:p><w:r><w:t>Name</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>Count</w:t></w:r></w:p></w:tc></w:tr>
      <w:tr><w:tc><w:p><w:r><w:t>Apples</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc></w:tr></w:tbl>
      <w:p><w:r><w:t>Final </w:t></w:r><w:r><w:t>paragraph</w:t></w:r><w:del><w:r><w:delText>removed</w:delText></w:r></w:del></w:p>
    </w:body></w:document>"#;
    let file = fs::File::create(dir.path().join("sample.docx")).unwrap();
    let mut archive = zip::ZipWriter::new(file);
    archive
        .start_file(
            "word/document.xml",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(xml.as_bytes()).unwrap();
    archive.finish().unwrap();
    let entry = store.convert(&request(dir.path(), "sample.docx")).unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(
        range.get((0, 0)),
        Some(&Data::String("Introduction".into()))
    );
    assert_eq!(range.get((1, 1)), Some(&Data::String("Count".into())));
    assert_eq!(range.get((2, 0)), Some(&Data::String("Apples".into())));
    assert_eq!(
        range.get((3, 0)),
        Some(&Data::String("Final paragraph".into()))
    );
    assert_eq!(entry.rows, 4);
}

#[test]
fn json_unions_object_keys_and_preserves_large_integer_precision() {
    let (dir, store) = ready();
    fs::write(dir.path().join("rows.json"), r#"[{"name":"One","value":1234567890123456789},{"extra":true,"name":"Two","nested":{"x":1}}]"#).unwrap();
    let entry = store.convert(&request(dir.path(), "rows.json")).unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(range.get((0, 0)), Some(&Data::String("extra".into())));
    assert_eq!(
        range.get((1, 3)),
        Some(&Data::String("1234567890123456789".into()))
    );
    assert_eq!(range.get((2, 0)), Some(&Data::Bool(true)));
    assert_eq!(range.get((2, 2)), Some(&Data::String(r#"{"x":1}"#.into())));
}

#[test]
fn source_workbook_preserves_values_and_sparse_offsets() {
    let (dir, store) = ready();
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let sheet = workbook.add_worksheet();
    sheet.set_name("Budget").unwrap();
    sheet.write_string(2, 2, "Item").unwrap();
    sheet.write_number(3, 3, 42.5).unwrap();
    workbook.save(dir.path().join("original.xlsx")).unwrap();
    let entry = store
        .convert(&request(dir.path(), "original.xlsx"))
        .unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let mut output = open_workbook_auto(entry.output_path.unwrap()).unwrap();
    assert_eq!(output.sheet_names(), &["Budget"]);
    let range = output.worksheet_range("Budget").unwrap();
    assert_eq!(range.get_value((2, 2)), Some(&Data::String("Item".into())));
    assert_eq!(range.get_value((3, 3)), Some(&Data::Float(42.5)));
    assert!(!entry.warnings.is_empty());
}

#[test]
fn format_protection_keeps_xlsx_merges_styles_and_formulas() {
    let (dir, store) = ready();
    let path = dir.path().join("formatted.xlsx");
    let mut workbook = rust_xlsxwriter::Workbook::new();
    let format = rust_xlsxwriter::Format::new()
        .set_bold()
        .set_background_color(rust_xlsxwriter::Color::Yellow);
    let sheet = workbook.add_worksheet();
    sheet
        .merge_range(1, 1, 1, 2, "Merged heading", &format)
        .unwrap();
    sheet.write_formula(2, 1, "=2+2").unwrap();
    workbook.save(&path).unwrap();
    let original = fs::read(&path).unwrap();

    let mut conversion = request(dir.path(), "formatted.xlsx");
    conversion.preserve_formatting = true;
    let entry = store.convert(&conversion).unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    assert_eq!(fs::read(entry.output_path.unwrap()).unwrap(), original);
    assert!(entry
        .warnings
        .iter()
        .any(|warning| warning.contains("Format Protection")));
}

#[test]
fn long_cells_and_corrupt_archives_fail_without_creating_workbooks() {
    let (dir, store) = ready();
    fs::write(dir.path().join("long.txt"), "x".repeat(32_768)).unwrap();
    let entry = store.convert(&request(dir.path(), "long.txt")).unwrap();
    assert_eq!(entry.status, "failed");
    assert!(entry.error.unwrap().contains("32,767"));
    fs::write(dir.path().join("bad.docx"), b"not a zip").unwrap();
    let entry = store.convert(&request(dir.path(), "bad.docx")).unwrap();
    assert_eq!(entry.status, "failed");
    assert!(entry.output_path.is_none());
    assert_eq!(
        fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "xlsx"))
            .count(),
        0
    );
}

#[test]
fn utf16_text_and_tsv_are_supported() {
    let (dir, store) = ready();
    let mut bytes = vec![0xff, 0xfe];
    bytes.extend(
        "Name\tCode\nZoë\t0012"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    fs::write(dir.path().join("unicode.tsv"), bytes).unwrap();
    let entry = store.convert(&request(dir.path(), "unicode.tsv")).unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(range.get((1, 0)), Some(&Data::String("Zoë".into())));
    assert_eq!(range.get((1, 1)), Some(&Data::String("0012".into())));
}

#[test]
fn future_database_schema_is_not_downgraded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.sqlite3");
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("PRAGMA user_version=99;").unwrap();
    assert!(Store::open(&path).is_err());
    let version: i32 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn selectable_pdf_text_is_extracted_with_page_numbers() {
    let (dir, store) = ready();
    fs::write(
        dir.path().join("sample.pdf"),
        simple_pdf("Hello PDF", "Second line"),
    )
    .unwrap();
    let entry = store.convert(&request(dir.path(), "sample.pdf")).unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(range.get((0, 0)), Some(&Data::String("Page".into())));
    let all_text = range
        .rows()
        .filter_map(|row| row.get(2))
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(all_text.contains("Hello PDF"), "{all_text}");
    assert!(all_text.contains("Second line"), "{all_text}");
    assert!(!entry.warnings.is_empty());
}

#[test]
fn positioned_pdf_words_are_grouped_into_visual_rows_in_both_review_modes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("positioned-words.pdf");
    fs::write(&path, positioned_words_pdf()).unwrap();
    for mode in [PdfReviewMode::SelectedPages, PdfReviewMode::DetectTables] {
        let preview = preview_file(&PreviewRequest {
            input_path: path.to_string_lossy().into_owned(),
            pdf_options: PdfReviewOptions {
                mode,
                page_selection: None,
            },
        })
        .unwrap();
        assert_eq!(preview.page_count, Some(1));
        assert_eq!(preview.selected_pages, vec![1]);
        assert_eq!(preview.detected_tables, 0);
        assert_eq!(preview.sheets[0].total_rows, 3);
        let rows = &preview.sheets[0].rows;
        assert_eq!(rows[1][1], PreviewCell::Number(1.0));
        assert_eq!(rows[2][1], PreviewCell::Number(2.0));
        assert_eq!(
            rows[1][2],
            PreviewCell::Text("Section VII. Technical Specifications".into())
        );
        assert_eq!(
            rows[2][2],
            PreviewCell::Text("Preparing the Technical Specifications".into())
        );
    }
}

#[test]
fn pdf_preview_reads_only_selected_pages_and_exports_reviewed_edits() {
    let (dir, store) = ready();
    fs::write(
        dir.path().join("pages.pdf"),
        two_page_pdf(
            &["FIRST PAGE", "Ignore this text"],
            &["SECOND PAGE", "Keep this text"],
        ),
    )
    .unwrap();
    let pdf_options = PdfReviewOptions {
        mode: PdfReviewMode::SelectedPages,
        page_selection: Some("2".into()),
    };
    let preview = preview_file(&PreviewRequest {
        input_path: dir.path().join("pages.pdf").to_string_lossy().into_owned(),
        pdf_options: pdf_options.clone(),
    })
    .unwrap();
    assert_eq!(preview.page_count, Some(2));
    assert_eq!(preview.selected_pages, vec![2]);
    let text = preview.sheets[0]
        .rows
        .iter()
        .flatten()
        .filter_map(|cell| match cell {
            PreviewCell::Text(value) => Some(value.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(text.contains("SECOND PAGE"), "{text}");
    assert!(!text.contains("FIRST PAGE"), "{text}");

    let entry = store
        .convert_reviewed(&ReviewedConversionRequest {
            input_path: dir.path().join("pages.pdf").to_string_lossy().into_owned(),
            output_directory: dir.path().to_string_lossy().into_owned(),
            source_fingerprint: preview.source_fingerprint,
            pdf_options,
            edits: vec![CellEdit {
                sheet_index: 0,
                row_index: 1,
                column_index: 2,
                value: "Corrected heading".into(),
            }],
            sheet_names: vec!["Reviewed pages".into()],
            preserve_formatting: false,
        })
        .unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let mut workbook = open_workbook_auto(entry.output_path.unwrap()).unwrap();
    assert_eq!(workbook.sheet_names(), &["Reviewed pages"]);
    let range = workbook.worksheet_range("Reviewed pages").unwrap();
    assert_eq!(
        range.get((1, 2)),
        Some(&Data::String("Corrected heading".into()))
    );
}

#[test]
fn pdf_table_detection_returns_candidates_for_review() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("table.pdf"),
        simple_pdf("Name|Amount", "Apples|12"),
    )
    .unwrap();
    let preview = preview_file(&PreviewRequest {
        input_path: dir.path().join("table.pdf").to_string_lossy().into_owned(),
        pdf_options: PdfReviewOptions {
            mode: PdfReviewMode::DetectTables,
            page_selection: None,
        },
    })
    .unwrap();
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.selected_pages, vec![1]);
    assert_eq!(preview.sheets[0].total_rows, 2);
    assert_eq!(preview.sheets[0].total_columns, 2);
}

#[test]
fn spacing_table_keeps_surrounding_paragraphs_and_empty_cells() {
    let (dir, store) = ready();
    let content = concat!(
        "BT /F1 12 Tf 72 720 Td ",
        "(Introduction paragraph) Tj 0 -18 Td ",
        "(Item|Description|Cost) Tj 0 -18 Td ",
        "(A||3) Tj 0 -18 Td ",
        "(Closing paragraph) Tj ET"
    );
    let path = dir.path().join("mixed-spacing.pdf");
    fs::write(&path, pdf_with_content(content)).unwrap();
    let options = PdfReviewOptions::default();
    let preview = preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: options.clone(),
    })
    .unwrap();
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.selected_pages, vec![1]);
    let rows = &preview.sheets[0].rows;
    assert_eq!(rows.len(), 4);
    assert!(rows[0].contains(&PreviewCell::Text("Introduction paragraph".into())));
    assert_eq!(
        rows[1],
        vec![
            PreviewCell::Text("Item".into()),
            PreviewCell::Text("Description".into()),
            PreviewCell::Text("Cost".into()),
        ]
    );
    assert_eq!(
        rows[2],
        vec![
            PreviewCell::Text("A".into()),
            PreviewCell::Text(String::new()),
            PreviewCell::Text("3".into()),
        ]
    );
    assert!(rows[3].contains(&PreviewCell::Text("Closing paragraph".into())));

    let entry = store
        .convert_reviewed(&ReviewedConversionRequest {
            input_path: path.to_string_lossy().into_owned(),
            output_directory: dir.path().to_string_lossy().into_owned(),
            source_fingerprint: preview.source_fingerprint,
            pdf_options: options,
            edits: Vec::new(),
            sheet_names: Vec::new(),
            preserve_formatting: false,
        })
        .unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(range.height(), 4);
    assert_eq!(range.get((2, 2)), Some(&Data::String("3".into())));

    let direct = store
        .convert(&request(dir.path(), "mixed-spacing.pdf"))
        .unwrap();
    assert_eq!(direct.status, "success", "{:?}", direct.error);
    let direct_range = read_first(direct.output_path.as_ref().unwrap());
    assert_eq!(direct_range.height(), 4);
    assert_eq!(direct_range.get((2, 2)), Some(&Data::String("3".into())));
}

#[test]
fn ruled_table_keeps_paragraphs_on_table_and_text_only_pages() {
    let (dir, store) = ready();
    let first = concat!(
        "0 g ",
        "60 650 0.5 100 re f 200 650 0.5 100 re f 400 650 0.5 100 re f 550 650 0.5 100 re f ",
        "60 650 490 0.5 re f 60 700 490 0.5 re f 60 750 490 0.5 re f ",
        "BT /F1 10 Tf ",
        "1 0 0 1 70 765 Tm (Opening paragraph) Tj ",
        "1 0 0 1 70 720 Tm (Item) Tj ",
        "1 0 0 1 210 720 Tm (Specification) Tj ",
        "1 0 0 1 410 720 Tm (Compliance) Tj ",
        "1 0 0 1 70 670 Tm (1) Tj ",
        "1 0 0 1 210 670 Tm (Wheeled Stretcher) Tj ",
        "1 0 0 1 410 670 Tm (NO BID) Tj ",
        "1 0 0 1 70 630 Tm (Follow-up paragraph) Tj ET"
    );
    let second = "BT /F1 12 Tf 72 720 Td (Continuation paragraph) Tj ET";
    let path = dir.path().join("mixed-ruled.pdf");
    fs::write(&path, pdf_with_two_contents(first, second)).unwrap();
    let preview = preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: PdfReviewOptions::default(),
    })
    .unwrap();
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.selected_pages, vec![1, 2]);
    let rows = &preview.sheets[0].rows;
    assert_eq!(rows.len(), 5);
    assert!(rows[0].contains(&PreviewCell::Text("Opening paragraph".into())));
    assert_eq!(rows[1][0], PreviewCell::Text("Item".into()));
    assert_eq!(rows[2][1], PreviewCell::Text("Wheeled Stretcher".into()));
    assert!(rows[3].contains(&PreviewCell::Text("Follow-up paragraph".into())));
    assert!(rows[4].contains(&PreviewCell::Text("Continuation paragraph".into())));

    let entry = store
        .convert(&request(dir.path(), "mixed-ruled.pdf"))
        .unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    assert_eq!(read_first(entry.output_path.as_ref().unwrap()).height(), 5);
}

#[test]
fn mixed_pdf_warns_when_a_page_has_no_selectable_text() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mixed-scan.pdf");
    fs::write(
        &path,
        pdf_with_two_contents("BT /F1 12 Tf 72 720 Td (Readable paragraph) Tj ET", ""),
    )
    .unwrap();
    let preview = preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: PdfReviewOptions::default(),
    })
    .unwrap();
    assert_eq!(preview.selected_pages, vec![1]);
    assert!(preview
        .warnings
        .iter()
        .any(|warning| warning.contains("Page 2 has no horizontal selectable text")));
}

#[test]
fn ruled_pdf_table_uses_layout_columns_instead_of_text_rows() {
    let (dir, store) = ready();
    fs::write(dir.path().join("ruled-table.pdf"), ruled_table_pdf()).unwrap();
    let preview = preview_file(&PreviewRequest {
        input_path: dir
            .path()
            .join("ruled-table.pdf")
            .to_string_lossy()
            .into_owned(),
        pdf_options: PdfReviewOptions {
            mode: PdfReviewMode::DetectTables,
            page_selection: None,
        },
    })
    .unwrap();
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.selected_pages, vec![1]);
    assert_eq!(preview.sheets[0].total_rows, 2);
    assert_eq!(preview.sheets[0].total_columns, 3);
    assert_eq!(
        preview.sheets[0].rows[0],
        vec![
            PreviewCell::Text("Item".into()),
            PreviewCell::Text("Specification".into()),
            PreviewCell::Text("Compliance".into()),
        ]
    );
    assert_eq!(
        preview.sheets[0].rows[1],
        vec![
            PreviewCell::Text("1".into()),
            PreviewCell::Text("Wheeled Stretcher".into()),
            PreviewCell::Text("NO BID".into()),
        ]
    );

    let entry = store
        .convert(&request(dir.path(), "ruled-table.pdf"))
        .unwrap();
    assert_eq!(entry.status, "success", "{:?}", entry.error);
    let range = read_first(entry.output_path.as_ref().unwrap());
    assert_eq!(range.height(), 2);
    assert_eq!(range.width(), 3);
    assert_eq!(range.get((0, 0)), Some(&Data::String("Item".into())));
    assert_eq!(
        range.get((1, 1)),
        Some(&Data::String("Wheeled Stretcher".into()))
    );
}

#[test]
fn decorated_pdf_table_keeps_cells_out_of_page_rules() {
    let (dir, _store) = ready();
    fs::write(
        dir.path().join("decorated-table.pdf"),
        decorated_ruled_table_pdf(),
    )
    .unwrap();
    let preview = preview_file(&PreviewRequest {
        input_path: dir
            .path()
            .join("decorated-table.pdf")
            .to_string_lossy()
            .into_owned(),
        pdf_options: PdfReviewOptions {
            mode: PdfReviewMode::DetectTables,
            page_selection: None,
        },
    })
    .unwrap();
    assert_eq!(preview.detected_tables, 1);
    assert_eq!(preview.sheets[0].total_rows, 2);
    assert_eq!(preview.sheets[0].total_columns, 3);
    assert_eq!(
        preview.sheets[0].rows[1],
        vec![
            PreviewCell::Text("1".into()),
            PreviewCell::Text("Wheeled Stretcher".into()),
            PreviewCell::Text("NO BID".into()),
        ]
    );
}

#[test]
fn reviewed_conversion_rejects_a_source_changed_after_preview() {
    let (dir, store) = ready();
    let path = dir.path().join("changing.pdf");
    fs::write(&path, simple_pdf("Original", "Content")).unwrap();
    let pdf_options = PdfReviewOptions::default();
    let preview = preview_file(&PreviewRequest {
        input_path: path.to_string_lossy().into_owned(),
        pdf_options: pdf_options.clone(),
    })
    .unwrap();
    fs::write(&path, simple_pdf("Changed", "Content")).unwrap();
    let entry = store
        .convert_reviewed(&ReviewedConversionRequest {
            input_path: path.to_string_lossy().into_owned(),
            output_directory: dir.path().to_string_lossy().into_owned(),
            source_fingerprint: preview.source_fingerprint,
            pdf_options,
            edits: Vec::new(),
            sheet_names: preview
                .sheets
                .iter()
                .map(|sheet| sheet.name.clone())
                .collect(),
            preserve_formatting: false,
        })
        .unwrap();
    assert_eq!(entry.status, "failed");
    assert!(entry
        .error
        .as_deref()
        .is_some_and(|error| error.contains("changed after it was previewed")));
    assert!(entry.output_path.is_none());
}

fn simple_pdf(first: &str, second: &str) -> Vec<u8> {
    let content = format!("BT /F1 12 Tf 72 720 Td ({first}) Tj 0 -18 Td ({second}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    output.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn positioned_words_pdf() -> Vec<u8> {
    let content = concat!(
        "BT /F1 12 Tf ",
        "1 0 0 1 72 720 Tm (Section) Tj ",
        "1 0 0 1 123 720 Tm (VII.) Tj ",
        "1 0 0 1 156 720 Tm (Technical) Tj ",
        "1 0 0 1 213 720 Tm (Specifications) Tj ",
        "1 0 0 1 72 700 Tm (Preparing) Tj ",
        "1 0 0 1 130 700 Tm (the) Tj ",
        "1 0 0 1 153 700 Tm (Technical) Tj ",
        "1 0 0 1 210 700 Tm (Specifications) Tj ET"
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    output.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn ruled_table_pdf() -> Vec<u8> {
    let content = concat!(
        "0 g ",
        "60 650 0.5 100 re f 200 650 0.5 100 re f 400 650 0.5 100 re f 550 650 0.5 100 re f ",
        "60 650 490 0.5 re f 60 700 490 0.5 re f 60 750 490 0.5 re f ",
        "BT /F1 10 Tf ",
        "1 0 0 1 70 720 Tm (Item) Tj ",
        "1 0 0 1 210 720 Tm (Specification) Tj ",
        "1 0 0 1 410 720 Tm (Compliance) Tj ",
        "1 0 0 1 70 670 Tm (1) Tj ",
        "1 0 0 1 210 670 Tm (Wheeled Stretcher) Tj ",
        "1 0 0 1 410 670 Tm (NO BID) Tj ET"
    );
    pdf_with_content(content)
}

fn decorated_ruled_table_pdf() -> Vec<u8> {
    let content = concat!(
        "0 g 0 699 612 93 re f ",
        "8 650 596 0.5 re f 8 675 596 0.5 re f 8 700 596 0.5 re f ",
        "8 725 596 0.5 re f 8 750 596 0.5 re f ",
        "60 650 490 100 re S ",
        "200 650 m 200 750 l S 400 650 m 400 750 l S ",
        "60 700 m 550 700 l S ",
        "BT /F1 10 Tf ",
        "1 0 0 1 70 720 Tm (Item) Tj ",
        "1 0 0 1 210 720 Tm (Specification) Tj ",
        "1 0 0 1 410 720 Tm (Compliance) Tj ",
        "1 0 0 1 70 670 Tm (1) Tj ",
        "1 0 0 1 210 670 Tm (Wheeled Stretcher) Tj ",
        "1 0 0 1 410 670 Tm (NO BID) Tj ET"
    );
    pdf_with_content(content)
}

fn pdf_with_content(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
    ];
    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    output.extend(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}

fn two_page_pdf(first_page: &[&str], second_page: &[&str]) -> Vec<u8> {
    fn content(lines: &[&str]) -> String {
        let commands = lines
            .iter()
            .enumerate()
            .map(|(index, line)| {
                if index == 0 {
                    format!("({line}) Tj")
                } else {
                    format!("0 -18 Td ({line}) Tj")
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!("BT /F1 12 Tf 72 720 Td {commands} ET")
    }
    let first = content(first_page);
    let second = content(second_page);
    pdf_with_two_contents(&first, &second)
}

fn pdf_with_two_contents(first: &str, second: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 7 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{first}\nendstream", first.len()),
        format!("<< /Length {} >>\nstream\n{second}\nendstream", second.len()),
    ];
    let mut output = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    output.extend(b"xref\n0 8\n0000000000 65535 f \n");
    for offset in offsets {
        output.extend(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend(
        format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    output
}
