use calamine::{open_workbook_auto, Data, Reader};
use sheetshift_core::{ConversionRequest, Settings, Store, TERMS_VERSION};
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
