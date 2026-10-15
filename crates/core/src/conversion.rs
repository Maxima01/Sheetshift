use crate::{supported_formats, ConversionRequest};
use anyhow::{bail, ensure, Context, Result};
use calamine::{open_workbook_auto_from_rs, Data, Reader};
use roxmltree::Node;
use rust_xlsxwriter::Workbook;
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use zip::ZipArchive;

// These limits reject oversized work instead of silently truncating documents.
const MAX_INPUT_BYTES: u64 = 50 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
const MAX_ZIP_ENTRIES: usize = 10_000;
const MAX_CELLS: usize = 500_000;
const MAX_ROWS: usize = 100_000;
const MAX_COLUMNS: usize = 16_384;
const MAX_SHEETS: usize = 128;
const MAX_CELL_CHARS: usize = 32_767;

pub(crate) struct ConversionResult {
    pub output_path: PathBuf,
    pub rows: u64,
    pub sheets: u32,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
enum Cell {
    Text(String),
    Number(f64),
    Boolean(bool),
    Empty,
}

struct Sheet {
    name: String,
    rows: Vec<Vec<Cell>>,
}

impl Sheet {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rows: Vec::new(),
        }
    }

    fn push(&mut self, row: Vec<Cell>, cells: &mut usize) -> Result<()> {
        ensure!(self.rows.len() < MAX_ROWS, "A worksheet exceeds the 100,000-row limit. Split the source document into smaller files.");
        ensure!(
            row.len() <= MAX_COLUMNS,
            "A row exceeds Excel's 16,384-column limit."
        );
        *cells = cells.saturating_add(row.len().max(1));
        ensure!(*cells <= MAX_CELLS, "The document exceeds the 500,000-cell limit. Split the source document into smaller files.");
        for cell in &row {
            if let Cell::Text(text) = cell {
                ensure!(text.chars().count() <= MAX_CELL_CHARS,
                    "A cell contains more than Excel's 32,767-character limit. Split long paragraphs or fields before converting.");
            }
        }
        self.rows.push(row);
        Ok(())
    }
}

pub(crate) fn convert(request: &ConversionRequest) -> Result<ConversionResult> {
    ensure!(
        !request.input_path.trim().is_empty(),
        "Choose a source file."
    );
    ensure!(
        !request.output_directory.trim().is_empty(),
        "Choose an output folder."
    );
    let source = Path::new(&request.input_path);
    let extension = source
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    ensure!(supported_formats().contains(&extension),
        "Unsupported file format '{}'. Supported formats: {}. For legacy .doc files, save as .docx first.",
        extension, supported_formats().join(", "));
    let file = File::open(source).with_context(|| {
        format!(
            "Could not open {}. Check that the file exists and is readable",
            source.display()
        )
    })?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "The source must be a regular file.");
    ensure!(
        metadata.len() <= MAX_INPUT_BYTES,
        "The source exceeds the 50 MiB file limit. Split it into smaller files."
    );
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_INPUT_BYTES,
        "The source grew beyond the 50 MiB file limit while reading."
    );
    ensure!(!bytes.is_empty(), "The source file is empty.");
    let output_dir = Path::new(&request.output_directory)
        .canonicalize()
        .context(
            "The output folder does not exist or cannot be accessed. Choose an existing folder.",
        )?;
    ensure!(
        output_dir.is_dir(),
        "The output path must be an existing folder."
    );

    if matches!(
        extension.as_str(),
        "docx" | "xlsx" | "xlsm" | "xlsb" | "ods"
    ) || (extension == "xls" && bytes.starts_with(b"PK"))
    {
        validate_zip(&bytes)?;
    }
    let mut warnings = Vec::new();
    let sheets = match extension.as_str() {
        "docx" => parse_docx(&bytes, &mut warnings)?,
        "pdf" => parse_pdf(&bytes, &mut warnings)?,
        "csv" => parse_delimited(&bytes, b',')?,
        "tsv" => parse_delimited(&bytes, b'\t')?,
        "txt" => parse_text(&bytes)?,
        "json" => parse_json(&bytes)?,
        _ => parse_workbook(&bytes, &mut warnings)?,
    };
    ensure!(
        !sheets.is_empty(),
        "No readable worksheets or text were found."
    );
    ensure!(
        sheets.len() <= MAX_SHEETS,
        "The source exceeds the 128-worksheet limit."
    );
    ensure!(
        sheets.iter().any(|s| !s.rows.is_empty()),
        "The source has no readable rows."
    );

    let rows = sheets.iter().map(|s| s.rows.len() as u64).sum();
    let sheet_count = sheets.len() as u32;
    let mut workbook = Workbook::new();
    let mut names = BTreeSet::new();
    for (index, sheet) in sheets.into_iter().enumerate() {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name(safe_sheet_name(&sheet.name, index, &mut names))?;
        // Estimate widths with bounded work and a sensible cap. General text
        // should be readable immediately, especially PDF's third text column.
        let mut widths = Vec::<usize>::new();
        for row in &sheet.rows {
            if widths.len() < row.len() {
                widths.resize(row.len(), 10);
            }
            for (column, cell) in row.iter().enumerate() {
                let length = match cell {
                    Cell::Text(text) => text
                        .lines()
                        .map(|line| line.chars().take(90).count())
                        .max()
                        .unwrap_or(0),
                    Cell::Number(value) => value.to_string().len(),
                    Cell::Boolean(_) => 5,
                    Cell::Empty => 0,
                };
                widths[column] = widths[column].max(length.saturating_add(2).min(90));
            }
        }
        for (column, width) in widths.iter().enumerate() {
            worksheet.set_column_width(column as u16, *width as f64)?;
        }
        if extension == "pdf" {
            worksheet.set_column_width(0, 8)?;
            worksheet.set_column_width(1, 8)?;
            worksheet.set_column_width(2, 90)?;
        } else if matches!(extension.as_str(), "docx" | "txt") {
            worksheet.set_column_width(0, widths.first().copied().unwrap_or(60).max(60) as f64)?;
        }
        for (row_index, row) in sheet.rows.into_iter().enumerate() {
            for (column_index, cell) in row.into_iter().enumerate() {
                let r = row_index as u32;
                let c = column_index as u16;
                match cell {
                    // Explicit strings prevent formula or URL auto-detection.
                    Cell::Text(text) => {
                        worksheet.write_string(r, c, &text)?;
                    }
                    Cell::Number(number) if number.is_finite() => {
                        worksheet.write_number(r, c, number)?;
                    }
                    Cell::Number(number) => {
                        worksheet.write_string(r, c, number.to_string())?;
                    }
                    Cell::Boolean(value) => {
                        worksheet.write_boolean(r, c, value)?;
                    }
                    Cell::Empty => {}
                }
            }
        }
    }
    let buffer = workbook
        .save_to_buffer()
        .context("Could not create the Excel workbook")?;
    let mut temporary = NamedTempFile::new_in(&output_dir)
        .context("Could not create a file in the output folder. Check its write permissions and available disk space")?;
    temporary
        .write_all(&buffer)
        .context("Could not write the workbook. Check available disk space")?;
    temporary.as_file().sync_all()?;
    let stem = safe_file_stem(
        source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref(),
    );
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            format!("{stem}_converted.xlsx")
        } else {
            format!("{stem}_converted ({suffix}).xlsx")
        };
        let target = output_dir.join(name);
        match temporary.persist_noclobber(&target) {
            Ok(_) => {
                return Ok(ConversionResult {
                    output_path: target,
                    rows,
                    sheets: sheet_count,
                    warnings,
                })
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file;
            }
            Err(error) => {
                return Err(error.error)
                    .context("Could not place the completed workbook in the output folder")
            }
        }
    }
    bail!("Too many output files share this name. Choose another output folder.")
}

fn validate_zip(bytes: &[u8]) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .context("This file is not a valid Office/OpenDocument archive. Re-export it from its original application")?;
    ensure!(
        archive.len() <= MAX_ZIP_ENTRIES,
        "The document archive has too many entries (limit: 10,000)."
    );
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .context("Cannot read an archive entry; encrypted documents are not supported")?;
        ensure!(
            entry.size() <= MAX_EXPANDED_BYTES.saturating_sub(expanded),
            "The expanded document exceeds the 128 MiB archive limit."
        );
        let remaining = MAX_EXPANDED_BYTES.saturating_sub(expanded);
        // Count actual bytes as well as the advertised size; do not trust ZIP metadata.
        let count = std::io::copy(&mut entry.take(remaining + 1), &mut std::io::sink())
            .context("A compressed document entry is corrupt or unreadable")?;
        expanded = expanded.saturating_add(count);
        ensure!(
            expanded <= MAX_EXPANDED_BYTES,
            "The expanded document exceeds the 128 MiB archive limit."
        );
    }
    Ok(())
}

fn decode_text(bytes: &[u8]) -> Result<String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        ensure!(
            (bytes.len() - 2).is_multiple_of(2),
            "The UTF-16 file contains an incomplete character."
        );
        let little = bytes[0] == 0xff;
        let words: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        return String::from_utf16(&words)
            .context("This file contains invalid UTF-16. Save it as UTF-8 and try again.");
    }
    let text = std::str::from_utf8(bytes).context("The text encoding is unsupported. Save the file as UTF-8 or UTF-16 with a BOM and try again.")?;
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

fn parse_delimited(bytes: &[u8], delimiter: u8) -> Result<Vec<Sheet>> {
    let text = decode_text(bytes)?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(text.as_bytes());
    let mut sheet = Sheet::new("Data");
    let mut cells = 0;
    for (index, record) in reader.records().enumerate() {
        let record =
            record.with_context(|| format!("Could not read delimited row {}", index + 1))?;
        sheet.push(
            record
                .iter()
                .map(|value| Cell::Text(value.to_owned()))
                .collect(),
            &mut cells,
        )?;
    }
    Ok(vec![sheet])
}

fn parse_text(bytes: &[u8]) -> Result<Vec<Sheet>> {
    let text = decode_text(bytes)?;
    ensure!(!text.trim().is_empty(), "The document contains no text.");
    let mut sheet = Sheet::new("Document");
    let mut cells = 0;
    for line in text.lines() {
        sheet.push(vec![Cell::Text(line.to_owned())], &mut cells)?;
    }
    Ok(vec![sheet])
}

fn parse_pdf(bytes: &[u8], warnings: &mut Vec<String>) -> Result<Vec<Sheet>> {
    let pages = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .context("Could not extract PDF text. The PDF may be encrypted, corrupt, or unsupported; try re-exporting an unencrypted PDF")?;
    ensure!(pages.iter().any(|page| !page.trim().is_empty()),
        "No selectable text was found in this PDF. Scanned/image-only PDFs require OCR, which this version does not include.");
    warnings.push("PDF text is extracted into Page, Line, and Text columns. Reading order may differ from the visual layout; tables, images, handwriting, and scanned pages are not reconstructed. Review the workbook against the source.".to_owned());
    let mut sheet = Sheet::new("Document");
    let mut cells = 0;
    sheet.push(
        vec![
            Cell::Text("Page".to_owned()),
            Cell::Text("Line".to_owned()),
            Cell::Text("Text".to_owned()),
        ],
        &mut cells,
    )?;
    for (page_index, page) in pages.iter().enumerate() {
        if page.trim().is_empty() {
            warnings.push(format!(
                "Page {} has no extractable text and may contain an image or scan.",
                page_index + 1
            ));
        }
        for (line_index, line) in page.lines().enumerate() {
            sheet.push(
                vec![
                    Cell::Number((page_index + 1) as f64),
                    Cell::Number((line_index + 1) as f64),
                    Cell::Text(line.to_owned()),
                ],
                &mut cells,
            )?;
        }
    }
    Ok(vec![sheet])
}

fn parse_docx(bytes: &[u8], warnings: &mut Vec<String>) -> Result<Vec<Sheet>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let mut xml = String::new();
    archive
        .by_name("word/document.xml")
        .context("The DOCX does not contain word/document.xml. Save it as a standard .docx file")?
        .take(MAX_EXPANDED_BYTES + 1)
        .read_to_string(&mut xml)
        .context("The DOCX document XML is not valid UTF-8")?;
    let document =
        roxmltree::Document::parse(&xml).context("The DOCX document XML is malformed")?;
    let body = document
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "body")
        .context("The DOCX document body is missing")?;
    let mut sheet = Sheet::new("Document");
    let mut cells = 0;
    collect_docx_blocks(body, &mut sheet, &mut cells)?;
    warnings.push("DOCX body paragraphs and table rows are kept in document order. Headers, footers, images, comments, tracked deletions, and formatting are excluded; merged and nested table cells are flattened.".to_owned());
    ensure!(
        sheet
            .rows
            .iter()
            .flatten()
            .any(|cell| matches!(cell, Cell::Text(text) if !text.trim().is_empty())),
        "The DOCX body contains no readable text or table values."
    );
    Ok(vec![sheet])
}

fn collect_docx_blocks(
    container: Node<'_, '_>,
    sheet: &mut Sheet,
    cells: &mut usize,
) -> Result<()> {
    for node in container.children().filter(Node::is_element) {
        match node.tag_name().name() {
            "p" => sheet.push(vec![Cell::Text(docx_text(node))], cells)?,
            "tbl" => {
                for row in node
                    .children()
                    .filter(|n| n.is_element() && n.tag_name().name() == "tr")
                {
                    let values = row
                        .children()
                        .filter(|n| n.is_element() && n.tag_name().name() == "tc")
                        .map(|cell| {
                            let paragraphs = cell
                                .descendants()
                                .filter(|n| n.is_element() && n.tag_name().name() == "p")
                                .map(docx_text)
                                .collect::<Vec<_>>();
                            Cell::Text(paragraphs.join("\n"))
                        })
                        .collect();
                    sheet.push(values, cells)?;
                }
            }
            // Content controls and inserted content can wrap paragraphs and tables.
            "sdt" | "sdtContent" | "customXml" | "ins" => collect_docx_blocks(node, sheet, cells)?,
            _ => {}
        }
    }
    Ok(())
}

fn docx_text(node: Node<'_, '_>) -> String {
    let mut text = String::new();
    for part in node.descendants().filter(Node::is_element) {
        if part
            .ancestors()
            .any(|ancestor| ancestor.is_element() && ancestor.tag_name().name() == "del")
        {
            continue;
        }
        match part.tag_name().name() {
            "t" => text.push_str(part.text().unwrap_or_default()),
            "tab" => text.push('\t'),
            "br" | "cr" => text.push('\n'),
            _ => {}
        }
    }
    text
}

fn parse_workbook(bytes: &[u8], warnings: &mut Vec<String>) -> Result<Vec<Sheet>> {
    let mut workbook = open_workbook_auto_from_rs(Cursor::new(bytes))
        .context("Could not read this spreadsheet. Check that it is not encrypted or corrupt")?;
    let names = workbook.sheet_names().to_owned();
    ensure!(
        names.len() <= MAX_SHEETS,
        "The source exceeds the 128-worksheet limit."
    );
    let mut sheets = Vec::new();
    let mut cells = 0;
    for name in names {
        let range = workbook
            .worksheet_range(&name)
            .with_context(|| format!("Could not read worksheet '{name}'"))?;
        let (height, width) = range.get_size();
        ensure!(
            height <= MAX_ROWS
                && width <= MAX_COLUMNS
                && height.saturating_mul(width) <= MAX_CELLS.saturating_sub(cells),
            "Worksheet '{name}' exceeds the row, column, or 500,000-cell document limit."
        );
        let mut sheet = Sheet::new(name);
        // Keep original row/column offsets instead of moving sparse cells to A1.
        let (start_row, start_column) = range.start().unwrap_or((0, 0));
        ensure!(
            start_row as usize + height <= MAX_ROWS && start_column as usize + width <= MAX_COLUMNS,
            "A source worksheet uses cells beyond the supported row or column limit."
        );
        for _ in 0..start_row {
            sheet.push(Vec::new(), &mut cells)?;
        }
        for row in range.rows() {
            let mut values = (0..start_column).map(|_| Cell::Empty).collect::<Vec<_>>();
            values.extend(row.iter().map(|value| match value {
                Data::Empty => Cell::Empty,
                Data::Int(number) => Cell::Number(*number as f64),
                Data::Float(number) => Cell::Number(*number),
                Data::Bool(boolean) => Cell::Boolean(*boolean),
                Data::String(text) => Cell::Text(text.clone()),
                // Dates and durations are rendered as source values rather than
                // accidentally applying a different workbook's epoch/format.
                _ => Cell::Text(value.to_string()),
            }));
            sheet.push(values, &mut cells)?;
        }
        sheets.push(sheet);
    }
    warnings.push("Spreadsheets are copied as cell values. Formulas use their stored results; formatting, charts, macros, links, merged cells, and validation are not preserved. Date/duration values are exported as text or source serial values. Hidden worksheets are included.".to_owned());
    Ok(sheets)
}

fn parse_json(bytes: &[u8]) -> Result<Vec<Sheet>> {
    let text = decode_text(bytes)?;
    let value: Value = serde_json::from_str(&text).context("This file is not valid JSON")?;
    let mut sheet = Sheet::new("Data");
    let mut cells = 0;
    match value {
        Value::Array(items) => {
            ensure!(!items.is_empty(), "The JSON array is empty.");
            if items.iter().all(Value::is_object) {
                let keys: BTreeSet<String> = items.iter().flat_map(|item| item.as_object().unwrap().keys().cloned()).collect();
                ensure!(!keys.is_empty(), "The JSON objects have no fields.");
                sheet.push(keys.iter().cloned().map(Cell::Text).collect(), &mut cells)?;
                for item in items {
                    sheet.push(keys.iter().map(|key| json_cell(item.get(key).unwrap_or(&Value::Null))).collect::<Result<Vec<_>>>()?, &mut cells)?;
                }
            } else if items.iter().all(Value::is_array) {
                for item in items {
                    sheet.push(item.as_array().unwrap().iter().map(json_cell).collect::<Result<Vec<_>>>()?, &mut cells)?;
                }
            } else {
                for item in items { sheet.push(vec![json_cell(&item)?], &mut cells)?; }
            }
        }
        Value::Object(values) => {
            ensure!(!values.is_empty(), "The JSON object has no fields.");
            sheet.push(vec![Cell::Text("Key".to_owned()), Cell::Text("Value".to_owned())], &mut cells)?;
            for (key, value) in values { sheet.push(vec![Cell::Text(key), json_cell(&value)?], &mut cells)?; }
        }
        _ => bail!("The JSON root must be an array or object. Wrap scalar values in an array before converting."),
    }
    Ok(vec![sheet])
}

fn json_cell(value: &Value) -> Result<Cell> {
    Ok(match value {
        Value::Null => Cell::Empty,
        Value::Bool(value) => Cell::Boolean(*value),
        Value::String(value) => Cell::Text(value.clone()),
        // Excel preserves about 15 significant digits. Keep large integers as
        // text to avoid silently rounding identifiers and account numbers.
        Value::Number(value)
            if value
                .to_string()
                .chars()
                .filter(char::is_ascii_digit)
                .count()
                <= 15 =>
        {
            value
                .as_f64()
                .filter(|n| n.is_finite())
                .map(Cell::Number)
                .unwrap_or_else(|| Cell::Text(value.to_string()))
        }
        Value::Number(value) => Cell::Text(value.to_string()),
        _ => Cell::Text(serde_json::to_string(value)?),
    })
}

fn safe_sheet_name(name: &str, index: usize, used: &mut BTreeSet<String>) -> String {
    let base: String = name
        .chars()
        .map(|ch| {
            if "[]:*?/\\".contains(ch) || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .take(31)
        .collect();
    let base = base.trim_matches('\'').trim();
    let base = if base.is_empty() || base.eq_ignore_ascii_case("History") {
        format!("Sheet {}", index + 1)
    } else {
        base.to_owned()
    };
    let mut candidate = base.clone();
    let mut suffix = 1;
    while !used.insert(candidate.to_lowercase()) {
        let end = format!(" ({suffix})");
        candidate = format!(
            "{}{}",
            base.chars().take(31 - end.len()).collect::<String>(),
            end
        );
        suffix += 1;
    }
    candidate
}

fn safe_file_stem(stem: &str) -> String {
    let result: String = stem
        .chars()
        .map(|ch| {
            if "<>:\"/\\|?*".contains(ch) || ch.is_control() {
                '_'
            } else {
                ch
            }
        })
        .take(80)
        .collect();
    let result = result.trim_matches([' ', '.']);
    if result.is_empty() {
        "document".to_owned()
    } else {
        result.to_owned()
    }
}
