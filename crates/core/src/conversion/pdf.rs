use super::{Cell, Sheet, MAX_SHEETS};
use crate::{PdfReviewMode, PdfReviewOptions};
use anyhow::{ensure, Context, Result};
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
struct PdfGlyph {
    text: String,
    x0: f64,
    x1: f64,
    top: f64,
    bottom: f64,
    font_size: f64,
    horizontal: bool,
}

#[derive(Clone, Copy, Debug)]
struct PdfRule {
    x0: f64,
    x1: f64,
    top: f64,
    bottom: f64,
}

#[derive(Debug)]
struct PdfLayoutPage {
    number: u32,
    llx: f64,
    ury: f64,
    height: f64,
    glyphs: Vec<PdfGlyph>,
    rules: Vec<PdfRule>,
}

#[derive(Default)]
struct PdfLayoutOutput {
    pages: Vec<PdfLayoutPage>,
    current_page: Option<usize>,
}

impl PdfLayoutOutput {
    fn add_rule_from_points(page: &mut PdfLayoutPage, first: (f64, f64), second: (f64, f64)) {
        let x0 = first.0.min(second.0) - page.llx;
        let x1 = first.0.max(second.0) - page.llx;
        let top = page.ury - first.1.max(second.1);
        let bottom = page.ury - first.1.min(second.1);
        if [x0, x1, top, bottom].iter().all(|value| value.is_finite()) {
            page.rules.push(PdfRule {
                x0,
                x1,
                top,
                bottom,
            });
        }
    }

    fn transformed_point(transform: &pdf_extract::Transform, x: f64, y: f64) -> (f64, f64) {
        (
            x * transform.m11 + y * transform.m21 + transform.m31,
            x * transform.m12 + y * transform.m22 + transform.m32,
        )
    }

    fn add_path(
        &mut self,
        transform: &pdf_extract::Transform,
        path: &pdf_extract::Path,
        is_stroke: bool,
    ) {
        let Some(page_index) = self.current_page else {
            return;
        };
        let page = &mut self.pages[page_index];
        let mut current = None;
        let mut start = None;
        for operation in &path.ops {
            match *operation {
                pdf_extract::PathOp::MoveTo(x, y) => {
                    let point = Self::transformed_point(transform, x, y);
                    current = Some(point);
                    start = Some(point);
                }
                pdf_extract::PathOp::LineTo(x, y) => {
                    let next = Self::transformed_point(transform, x, y);
                    if let Some(previous) = current {
                        Self::add_rule_from_points(page, previous, next);
                    }
                    current = Some(next);
                }
                pdf_extract::PathOp::Rect(x, y, width, height) => {
                    let corners = [
                        Self::transformed_point(transform, x, y),
                        Self::transformed_point(transform, x + width, y),
                        Self::transformed_point(transform, x + width, y + height),
                        Self::transformed_point(transform, x, y + height),
                    ];
                    if is_stroke {
                        for index in 0..4 {
                            Self::add_rule_from_points(
                                page,
                                corners[index],
                                corners[(index + 1) % 4],
                            );
                        }
                    } else {
                        // Filled rectangles are often backgrounds. Only thin ones
                        // represent a visible horizontal or vertical rule.
                        let min_x = corners
                            .iter()
                            .map(|point| point.0)
                            .fold(f64::INFINITY, f64::min);
                        let max_x = corners
                            .iter()
                            .map(|point| point.0)
                            .fold(f64::NEG_INFINITY, f64::max);
                        let min_y = corners
                            .iter()
                            .map(|point| point.1)
                            .fold(f64::INFINITY, f64::min);
                        let max_y = corners
                            .iter()
                            .map(|point| point.1)
                            .fold(f64::NEG_INFINITY, f64::max);
                        if max_x - min_x <= 2.5 || max_y - min_y <= 2.5 {
                            Self::add_rule_from_points(page, (min_x, min_y), (max_x, max_y));
                        }
                    }
                    current = None;
                    start = None;
                }
                pdf_extract::PathOp::Close => {
                    if let (Some(previous), Some(first)) = (current, start) {
                        Self::add_rule_from_points(page, previous, first);
                    }
                    current = start;
                }
                pdf_extract::PathOp::CurveTo(_, _, _, _, x, y) => {
                    current = Some(Self::transformed_point(transform, x, y));
                }
            }
        }
    }
}

impl pdf_extract::OutputDev for PdfLayoutOutput {
    fn begin_page(
        &mut self,
        page_num: u32,
        media_box: &pdf_extract::MediaBox,
        _art_box: Option<(f64, f64, f64, f64)>,
    ) -> std::result::Result<(), pdf_extract::OutputError> {
        self.pages.push(PdfLayoutPage {
            number: page_num,
            llx: media_box.llx,
            ury: media_box.ury,
            height: media_box.ury - media_box.lly,
            glyphs: Vec::new(),
            rules: Vec::new(),
        });
        self.current_page = Some(self.pages.len() - 1);
        Ok(())
    }

    fn end_page(&mut self) -> std::result::Result<(), pdf_extract::OutputError> {
        self.current_page = None;
        Ok(())
    }

    fn output_character(
        &mut self,
        transform: &pdf_extract::Transform,
        width: f64,
        _spacing: f64,
        font_size: f64,
        character: &str,
    ) -> std::result::Result<(), pdf_extract::OutputError> {
        let Some(page_index) = self.current_page else {
            return Ok(());
        };
        if character.is_empty() {
            return Ok(());
        }
        let page = &mut self.pages[page_index];
        // The callback's transform includes horizontal scaling, but not the
        // font size. Width and height need their own transformed axes: using
        // their geometric mean loses spaces in condensed text.
        let rendered_font_size = (font_size * transform.m21.hypot(transform.m22))
            .abs()
            .max(1.0);
        let corners = [
            Self::transformed_point(transform, 0.0, 0.0),
            Self::transformed_point(transform, width * font_size, 0.0),
            Self::transformed_point(transform, 0.0, font_size),
            Self::transformed_point(transform, width * font_size, font_size),
        ];
        let x0 = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::INFINITY, f64::min)
            - page.llx;
        let x1 = corners
            .iter()
            .map(|point| point.0)
            .fold(f64::NEG_INFINITY, f64::max)
            - page.llx;
        let top = page.ury
            - corners
                .iter()
                .map(|point| point.1)
                .fold(f64::NEG_INFINITY, f64::max);
        let bottom = page.ury
            - corners
                .iter()
                .map(|point| point.1)
                .fold(f64::INFINITY, f64::min);
        if [x0, x1, top, bottom].iter().all(|value| value.is_finite()) {
            page.glyphs.push(PdfGlyph {
                text: character.to_owned(),
                x0,
                x1,
                top,
                bottom,
                font_size: rendered_font_size,
                // Diagonal stamps and vertical labels otherwise get mixed into
                // unrelated horizontal rows and table cells.
                horizontal: transform.m11 > 0.0
                    && transform.m12.abs() <= transform.m11.abs() * 0.08,
            });
        }
        Ok(())
    }

    fn begin_word(&mut self) -> std::result::Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn end_word(&mut self) -> std::result::Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn end_line(&mut self) -> std::result::Result<(), pdf_extract::OutputError> {
        Ok(())
    }

    fn stroke(
        &mut self,
        transform: &pdf_extract::Transform,
        _colorspace: &pdf_extract::ColorSpace,
        _color: &[f64],
        path: &pdf_extract::Path,
    ) -> std::result::Result<(), pdf_extract::OutputError> {
        self.add_path(transform, path, true);
        Ok(())
    }

    fn fill(
        &mut self,
        transform: &pdf_extract::Transform,
        _colorspace: &pdf_extract::ColorSpace,
        _color: &[f64],
        path: &pdf_extract::Path,
    ) -> std::result::Result<(), pdf_extract::OutputError> {
        self.add_path(transform, path, false);
        Ok(())
    }
}

#[derive(Debug)]
struct PdfTextLine {
    text: String,
    left: f64,
    font_size: f64,
    top: f64,
    bottom: f64,
}

#[derive(Debug)]
struct PdfDetectedTable {
    column_boundaries: Vec<f64>,
    top: f64,
    bottom: f64,
    rows: Vec<Vec<String>>,
}

struct PdfRuledExtraction {
    sheets: Vec<Sheet>,
    included_pages: Vec<u32>,
    detected_tables: usize,
}

pub(super) struct PdfParseResult {
    pub sheets: Vec<Sheet>,
    pub page_count: u32,
    pub included_pages: Vec<u32>,
    pub detected_tables: usize,
}

fn extract_pdf_layout(
    bytes: &[u8],
    page_selection: Option<&str>,
) -> Result<(Vec<PdfLayoutPage>, u32, Vec<u32>)> {
    let mut document = pdf_extract::Document::load_mem(bytes)
        .context("Could not read PDF text positions or drawing commands")?;
    if document.is_encrypted() {
        document.decrypt("").context(
            "This PDF is password protected. Save an unencrypted copy before reviewing it",
        )?;
    }
    let page_count = document.get_pages().len() as u32;
    ensure!(page_count > 0, "The PDF contains no pages.");
    let selected_pages = if let Some(selection) = page_selection {
        parse_page_selection(Some(selection), page_count)?
    } else {
        (1..=page_count).collect()
    };
    let mut output = PdfLayoutOutput::default();
    if page_selection.is_some() {
        for page_number in &selected_pages {
            pdf_extract::output_doc_page(&document, &mut output, *page_number)
                .with_context(|| format!("Could not read positioned text on page {page_number}"))?;
        }
    } else {
        pdf_extract::output_doc(&document, &mut output)
            .context("Could not extract positioned text from the PDF")?;
    }
    Ok((output.pages, page_count, selected_pages))
}

fn clustered_positions(mut values: Vec<f64>, tolerance: f64) -> Vec<f64> {
    values.retain(|value| value.is_finite());
    values.sort_by(f64::total_cmp);
    let mut clusters = Vec::<(f64, usize)>::new();
    for value in values {
        if let Some((average, count)) = clusters.last_mut() {
            if (value - *average).abs() <= tolerance {
                *count += 1;
                *average += (value - *average) / *count as f64;
                continue;
            }
        }
        clusters.push((value, 1));
    }
    clusters.into_iter().map(|(average, _)| average).collect()
}

fn interval_coverage(mut intervals: Vec<(f64, f64)>) -> f64 {
    if intervals.is_empty() {
        return 0.0;
    }
    intervals.sort_by(|left, right| left.0.total_cmp(&right.0));
    let mut coverage = 0.0;
    let mut current = intervals[0];
    for interval in intervals.into_iter().skip(1) {
        if interval.0 <= current.1 + 1.5 {
            current.1 = current.1.max(interval.1);
        } else {
            coverage += (current.1 - current.0).max(0.0);
            current = interval;
        }
    }
    coverage + (current.1 - current.0).max(0.0)
}

fn vertical_coverage(page: &PdfLayoutPage, x: f64, top: f64, bottom: f64) -> f64 {
    interval_coverage(
        page.rules
            .iter()
            .filter(|rule| {
                rule.x1 - rule.x0 <= 2.5
                    && rule.bottom - rule.top >= 1.0
                    && ((rule.x0 + rule.x1) / 2.0 - x).abs() <= 1.5
            })
            .filter_map(|rule| {
                let start = rule.top.max(top);
                let end = rule.bottom.min(bottom);
                (end > start).then_some((start, end))
            })
            .collect(),
    )
}

fn horizontal_coverage(page: &PdfLayoutPage, y: f64, left: f64, right: f64) -> f64 {
    interval_coverage(
        page.rules
            .iter()
            .filter(|rule| {
                rule.bottom - rule.top <= 2.5
                    && rule.x1 - rule.x0 >= 5.0
                    && ((rule.top + rule.bottom) / 2.0 - y).abs() <= 1.5
            })
            .filter_map(|rule| {
                let start = rule.x0.max(left);
                let end = rule.x1.min(right);
                (end > start).then_some((start, end))
            })
            .collect(),
    )
}

fn text_lines<'a>(glyphs: impl IntoIterator<Item = &'a PdfGlyph>) -> Vec<PdfTextLine> {
    let mut glyphs = glyphs
        .into_iter()
        .filter(|glyph| glyph.horizontal)
        .collect::<Vec<_>>();
    glyphs.sort_by(|left, right| {
        left.bottom
            .total_cmp(&right.bottom)
            .then_with(|| left.x0.total_cmp(&right.x0))
    });
    let mut grouped = Vec::<Vec<&PdfGlyph>>::new();
    for glyph in glyphs {
        if let Some(line) = grouped.last_mut() {
            let baseline = line[0].bottom;
            let reference_size = line[0].font_size.min(glyph.font_size).max(1.0);
            let tolerance = reference_size * 0.35 + 0.75;
            if (glyph.bottom - baseline).abs() <= tolerance {
                line.push(glyph);
                continue;
            }
        }
        grouped.push(vec![glyph]);
    }

    grouped
        .into_iter()
        .filter_map(|mut line| {
            line.sort_by(|left, right| left.x0.total_cmp(&right.x0));
            // Some generators paint a glyph more than once for a bold or
            // shadow effect. Remove only matching glyphs at the same position,
            // never repeated letters at different positions.
            let mut unique = Vec::<&PdfGlyph>::with_capacity(line.len());
            for glyph in line {
                let duplicate = unique
                    .iter()
                    .rev()
                    .take_while(|previous| glyph.x0 - previous.x0 <= 0.25)
                    .any(|previous| {
                        glyph.text == previous.text
                            && (glyph.x1 - previous.x1).abs() <= 0.25
                            && (glyph.bottom - previous.bottom).abs() <= 0.25
                            && (glyph.font_size - previous.font_size).abs() <= 0.25
                    });
                if !duplicate {
                    unique.push(glyph);
                }
            }
            let line = unique;
            let mut text = String::new();
            let mut previous_end = None;
            for glyph in &line {
                if let Some(end) = previous_end {
                    let gap = glyph.x0 - end;
                    if gap > glyph.font_size * 2.0 && !text.ends_with(char::is_whitespace) {
                        text.push_str("  ");
                    } else if gap > glyph.font_size * 0.18 && !text.ends_with(char::is_whitespace) {
                        text.push(' ');
                    }
                }
                text.push_str(&glyph.text);
                previous_end = Some(glyph.x1);
            }
            let text = text.trim().to_owned();
            if text.is_empty() {
                return None;
            }
            Some(PdfTextLine {
                text,
                left: line
                    .iter()
                    .map(|glyph| glyph.x0)
                    .fold(f64::INFINITY, f64::min),
                font_size: line.iter().map(|glyph| glyph.font_size).fold(1.0, f64::max),
                top: line
                    .iter()
                    .map(|glyph| glyph.top)
                    .fold(f64::INFINITY, f64::min),
                bottom: line
                    .iter()
                    .map(|glyph| glyph.bottom)
                    .fold(f64::NEG_INFINITY, f64::max),
            })
        })
        .collect()
}

fn text_in_box(page: &PdfLayoutPage, left: f64, right: f64, top: f64, bottom: f64) -> String {
    text_lines(page.glyphs.iter().filter(|glyph| {
        let center_x = (glyph.x0 + glyph.x1) / 2.0;
        let center_y = (glyph.top + glyph.bottom) / 2.0;
        // Half-open bounds give each glyph one owner, including at shared
        // borders. Expanding both neighbors duplicated boundary characters.
        center_x >= left && center_x < right && center_y >= top && center_y < bottom
    }))
    .into_iter()
    .map(|line| line.text)
    .collect::<Vec<_>>()
    .join("\n")
}

fn joined_vertical_rules(page: &PdfLayoutPage) -> Vec<PdfRule> {
    let vertical = page
        .rules
        .iter()
        .filter(|rule| rule.x1 - rule.x0 <= 2.5 && rule.bottom - rule.top >= 1.0)
        .collect::<Vec<_>>();
    let positions = clustered_positions(
        vertical
            .iter()
            .map(|rule| (rule.x0 + rule.x1) / 2.0)
            .collect(),
        1.5,
    );
    let mut joined = Vec::<PdfRule>::new();
    for x in positions {
        let mut segments = vertical
            .iter()
            .filter(|rule| ((rule.x0 + rule.x1) / 2.0 - x).abs() <= 1.5)
            .map(|rule| (rule.top, rule.bottom))
            .collect::<Vec<_>>();
        segments.sort_by(|left, right| left.0.total_cmp(&right.0));
        let mut current: Option<(f64, f64)> = None;
        for (top, bottom) in segments {
            if let Some(span) = current.as_mut() {
                if top <= span.1 + 2.5 {
                    span.1 = span.1.max(bottom);
                    continue;
                }
                joined.push(PdfRule {
                    x0: x,
                    x1: x,
                    top: span.0,
                    bottom: span.1,
                });
            }
            current = Some((top, bottom));
        }
        if let Some((top, bottom)) = current {
            joined.push(PdfRule {
                x0: x,
                x1: x,
                top,
                bottom,
            });
        }
    }
    joined
}

fn detect_ruled_table(page: &PdfLayoutPage) -> Option<PdfDetectedTable> {
    // Find a local band supported by several vertical borders. Using the full
    // page makes decorative horizontal lines look like table row boundaries.
    // Word processors often draw each cell side as a separate thin rectangle.
    // Join adjacent pieces before selecting a band, otherwise the largest
    // individual cell becomes the entire table and other rows become prose.
    let joined = joined_vertical_rules(page);
    let vertical_rules = joined
        .iter()
        .filter(|rule| rule.bottom - rule.top >= 20.0)
        .collect::<Vec<_>>();
    let mut best: Option<(f64, f64, Vec<f64>, f64)> = None;
    for seed in &vertical_rules {
        let top = seed.top;
        let bottom = seed.bottom;
        let height = bottom - top;
        let positions = clustered_positions(
            vertical_rules
                .iter()
                .map(|rule| (rule.x0 + rule.x1) / 2.0)
                .collect(),
            1.5,
        )
        .into_iter()
        .filter(|x| vertical_coverage(page, *x, top, bottom) >= height * 0.5)
        .collect::<Vec<_>>();
        if positions.len() < 3 {
            continue;
        }
        let score = positions.len() as f64 * height;
        if best.as_ref().is_none_or(|candidate| score > candidate.3) {
            best = Some((top, bottom, positions, score));
        }
    }
    let (top, bottom, vertical_positions, _) = best?;
    let left = *vertical_positions.first()?;
    let right = *vertical_positions.last()?;
    if right - left < 80.0 {
        return None;
    }

    let mut horizontal_positions = clustered_positions(
        page.rules
            .iter()
            .filter(|rule| rule.bottom - rule.top <= 2.5 && rule.x1 - rule.x0 >= 5.0)
            .filter(|rule| {
                (rule.top + rule.bottom) / 2.0 >= top - 2.5
                    && (rule.top + rule.bottom) / 2.0 <= bottom + 2.5
            })
            .map(|rule| (rule.top + rule.bottom) / 2.0)
            .collect(),
        1.5,
    )
    .into_iter()
    .filter(|y| {
        let mut aligned = page.rules.iter().filter(|rule| {
            rule.bottom - rule.top <= 2.5 && ((rule.top + rule.bottom) / 2.0 - y).abs() <= 1.5
        });
        let left_edge = aligned.clone().any(|rule| (rule.x0 - left).abs() <= 3.0);
        let right_edge = aligned.any(|rule| (rule.x1 - right).abs() <= 3.0);
        left_edge
            && right_edge
            && horizontal_coverage(page, *y, left, right) >= (right - left) * 0.65
    })
    .collect::<Vec<_>>();
    if horizontal_positions.len() < 2 {
        return None;
    }
    horizontal_positions.sort_by(f64::total_cmp);

    let mut rows = Vec::new();
    let header_band_top = top - (horizontal_positions[1] - top).clamp(24.0, 40.0);
    let header = text_lines(page.glyphs.iter().filter(|glyph| {
        let center = (glyph.top + glyph.bottom) / 2.0;
        center >= header_band_top && center < top
    }))
    .into_iter()
    .filter_map(|line| {
        let cells = vertical_positions
            .windows(2)
            .map(|columns| {
                text_in_box(
                    page,
                    columns[0],
                    columns[1],
                    line.top - 0.5,
                    line.bottom + 0.5,
                )
            })
            .collect::<Vec<_>>();
        let filled = cells.iter().filter(|cell| !cell.is_empty()).count();
        (filled >= 2).then_some((filled, line.top, cells))
    })
    .max_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.total_cmp(&right.1))
    });
    let table_top = if let Some((_, header_top, header_cells)) = header {
        rows.push(header_cells);
        header_top - 1.0
    } else {
        top
    };
    for bounds in horizontal_positions.windows(2) {
        if bounds[1] - bounds[0] < 2.0 {
            continue;
        }
        let row = vertical_positions
            .windows(2)
            .map(|columns| text_in_box(page, columns[0], columns[1], bounds[0], bounds[1]))
            .collect::<Vec<_>>();
        if row.iter().any(|value| !value.is_empty()) {
            rows.push(row);
        }
    }
    (!rows.is_empty()).then_some(PdfDetectedTable {
        column_boundaries: vertical_positions,
        top: table_top,
        bottom,
        rows,
    })
}

fn context_rows_between(page: &PdfLayoutPage, top_limit: f64, bottom_limit: f64) -> Vec<String> {
    let mut lines = text_lines(page.glyphs.iter().filter(|glyph| {
        let center_y = (glyph.top + glyph.bottom) / 2.0;
        center_y > top_limit && center_y < bottom_limit
    }));
    lines.retain(|line| {
        !(line.text.trim().parse::<u32>().is_ok()
            && (line.top < page.height * 0.05 || line.bottom > page.height * 0.9))
    });
    // Keep wrapped prose together instead of turning every PDF baseline into a
    // spreadsheet record. Gaps, indentation and font changes separate blocks.
    let mut blocks = Vec::<String>::new();
    let mut previous: Option<&PdfTextLine> = None;
    for line in &lines {
        let continues = previous.is_some_and(|previous| {
            let size = previous.font_size.min(line.font_size);
            line.top - previous.bottom <= size * 0.65
                && (line.font_size - previous.font_size).abs() <= size * 0.1
                && (line.left - previous.left).abs() <= size * 0.8
                && blocks.last().is_some_and(|block| {
                    block.chars().count() + line.text.chars().count() < super::MAX_CELL_CHARS
                })
        });
        if continues {
            let block = blocks.last_mut().unwrap();
            block.push('\n');
            block.push_str(&line.text);
        } else {
            blocks.push(line.text.clone());
        }
        previous = Some(line);
    }
    blocks
}

fn context_cells(value: String, columns: usize) -> Vec<Cell> {
    let mut row = (0..columns.max(1)).map(|_| Cell::Empty).collect::<Vec<_>>();
    let text_column = if columns >= 2 { 1 } else { 0 };
    if !value.is_empty() {
        row[text_column] = Cell::Text(value);
    }
    row
}

fn extract_ruled_pdf_tables(pages: &[PdfLayoutPage]) -> Result<PdfRuledExtraction> {
    let tables = pages.iter().map(detect_ruled_table).collect::<Vec<_>>();
    let Some(first_table_index) = tables.iter().position(Option::is_some) else {
        return Ok(PdfRuledExtraction {
            sheets: Vec::new(),
            included_pages: Vec::new(),
            detected_tables: 0,
        });
    };

    let mut sheets = Vec::new();
    let mut included_pages = Vec::new();
    let mut current = Sheet::new("Table 1");
    let mut current_columns = tables[first_table_index]
        .as_ref()
        .map(|table| table.column_boundaries.len() - 1)
        .unwrap_or(1);
    let mut cells = 0usize;
    let mut previous_table: Option<(u32, Vec<f64>)> = None;

    for (page, table) in pages.iter().zip(tables.iter()) {
        if let Some(table) = table {
            included_pages.push(page.number);
            let columns = table.column_boundaries.len() - 1;
            let same_table = previous_table.as_ref().is_none_or(|(number, boundaries)| {
                page.number == number + 1
                    && boundaries.len() == table.column_boundaries.len()
                    && boundaries
                        .iter()
                        .zip(&table.column_boundaries)
                        .all(|(left, right)| (left - right).abs() <= 3.0)
            });
            if current_columns != columns || !same_table {
                if !current.rows.is_empty() {
                    sheets.push(current);
                    ensure!(
                        sheets.len() < MAX_SHEETS,
                        "More than 128 tables were found. Select fewer pages."
                    );
                }
                current = Sheet::new(format!("Table {}", sheets.len() + 1));
            }
            current_columns = columns;
            previous_table = Some((page.number, table.column_boundaries.clone()));
            for value in context_rows_between(page, 0.0, table.top) {
                current.push(context_cells(value, current_columns), &mut cells)?;
            }
            for row in &table.rows {
                current.push(row.iter().cloned().map(Cell::Text).collect(), &mut cells)?;
            }
            for value in context_rows_between(page, table.bottom, page.height) {
                current.push(context_cells(value, current_columns), &mut cells)?;
            }
        } else {
            let context = context_rows_between(page, 0.0, page.height);
            if !context.is_empty() {
                included_pages.push(page.number);
            }
            for value in context {
                current.push(context_cells(value, current_columns), &mut cells)?;
            }
        }
    }
    if !current.rows.is_empty() {
        sheets.push(current);
    }
    let detected_tables = sheets.len();
    Ok(PdfRuledExtraction {
        sheets,
        included_pages,
        detected_tables,
    })
}

fn warn_about_rotated_text(pages: &[PdfLayoutPage], warnings: &mut Vec<String>) {
    if pages
        .iter()
        .any(|page| page.glyphs.iter().any(|glyph| !glyph.horizontal))
    {
        warnings.push("Rotated or diagonal PDF text was excluded from horizontal row grouping. Compare labels and stamps with the source.".to_owned());
    }
}

fn warn_about_unreadable_pages(pages: &[PdfLayoutPage], warnings: &mut Vec<String>) {
    for page in pages {
        if text_lines(&page.glyphs).is_empty() {
            warnings.push(format!(
                "Page {} has no horizontal selectable text. It may be an image, a scan, or rotated text.",
                page.number
            ));
        }
    }
}

fn pdf_text_sheet(pages: &[PdfLayoutPage]) -> Result<Sheet> {
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
    let mut readable_pages = 0;
    for page in pages {
        let lines = text_lines(&page.glyphs);
        if lines.is_empty() {
            continue;
        }
        readable_pages += 1;
        for (line_index, line) in lines.into_iter().enumerate() {
            sheet.push(
                vec![
                    Cell::Number(page.number as f64),
                    Cell::Number((line_index + 1) as f64),
                    Cell::Text(line.text),
                ],
                &mut cells,
            )?;
        }
    }
    ensure!(
        readable_pages > 0,
        "No horizontal selectable text was found in this PDF. Image-only pages require OCR; rotated-only text needs a different extraction mode."
    );
    Ok(sheet)
}

pub(super) fn parse(bytes: &[u8], warnings: &mut Vec<String>) -> Result<Vec<Sheet>> {
    Ok(parse_for_review(bytes, &PdfReviewOptions::default(), warnings)?.sheets)
}

pub(super) fn parse_for_review(
    bytes: &[u8],
    options: &PdfReviewOptions,
    warnings: &mut Vec<String>,
) -> Result<PdfParseResult> {
    // Page selection controls scope, not extraction quality. Both review modes
    // and direct conversion must use the same structure-first pipeline.
    let selection = (options.mode == PdfReviewMode::SelectedPages)
        .then(|| options.page_selection.as_deref().unwrap_or(""));
    let (pages, page_count, selected_pages) = extract_pdf_layout(bytes, selection)?;
    if selection.is_some() {
        warnings.push(format!(
            "{} of {page_count} PDF pages selected. Tables on these pages are reconstructed from text positions and cell borders.",
            selected_pages.len()
        ));
    }
    let mut parsed = extract_layout_for_review(&pages, page_count, warnings)?;
    if selection.is_some() {
        parsed.included_pages = selected_pages;
    }
    Ok(parsed)
}

fn extract_layout_for_review(
    pages: &[PdfLayoutPage],
    page_count: u32,
    warnings: &mut Vec<String>,
) -> Result<PdfParseResult> {
    warn_about_rotated_text(pages, warnings);
    warn_about_unreadable_pages(pages, warnings);
    let ruled = extract_ruled_pdf_tables(pages)?;
    if !ruled.sheets.is_empty() {
        warnings.push(format!(
                    "Found {} ruled table{} across {} page{}. Detection used positioned text and cell borders; verify wrapped text, merged cells, and rows that continue across page breaks.",
                    ruled.detected_tables,
                    if ruled.detected_tables == 1 { "" } else { "s" },
                    ruled.included_pages.len(),
                    if ruled.included_pages.len() == 1 { "" } else { "s" }
                ));
        return Ok(PdfParseResult {
            sheets: ruled.sheets,
            page_count,
            included_pages: ruled.included_pages,
            detected_tables: ruled.detected_tables,
        });
    }
    let mut sheets = Vec::new();
    let mut included_pages = BTreeSet::new();
    let mut pending_context = Vec::new();
    let mut last_columns = 1;
    let mut cells = 0usize;
    for page in pages {
        let lines = text_lines(&page.glyphs)
            .into_iter()
            .map(|line| line.text)
            .collect::<Vec<_>>();
        if !lines.is_empty() {
            included_pages.insert(page.number);
        }
        let mut previous_end = 0;
        for block in detect_table_blocks(&lines) {
            ensure!(
                        sheets.len() < MAX_SHEETS,
                        "More than 128 table candidates were found. Select pages manually to narrow the review."
                    );
            let page_number = page.number;
            pending_context.extend(lines[previous_end..block.start].iter().cloned());
            last_columns = block.rows.iter().map(Vec::len).max().unwrap_or(1);
            let mut sheet = Sheet::new(format!("Table {} - page {page_number}", sheets.len() + 1));
            for value in pending_context.drain(..) {
                sheet.push(context_cells(value, last_columns), &mut cells)?;
            }
            for row in block.rows {
                sheet.push(row.into_iter().map(Cell::Text).collect(), &mut cells)?;
            }
            sheets.push(sheet);
            previous_end = block.end;
        }
        pending_context.extend(lines[previous_end..].iter().cloned());
    }
    if sheets.is_empty() {
        warnings.push("The PDF has selectable text, but no table rows could be identified. Showing its visual text rows for review.".to_owned());
        let included_pages = pages
            .iter()
            .filter(|page| !text_lines(&page.glyphs).is_empty())
            .map(|page| page.number)
            .collect();
        return Ok(PdfParseResult {
            sheets: vec![pdf_text_sheet(pages)?],
            page_count,
            included_pages,
            detected_tables: 0,
        });
    }
    for value in pending_context {
        sheets
            .last_mut()
            .unwrap()
            .push(context_cells(value, last_columns), &mut cells)?;
    }
    let detected_tables = sheets.len();
    warnings.push(format!(
                "Found {detected_tables} spacing-based table candidate{} across {} page{}. Surrounding text was kept in reading order; verify columns and headings before export.",
                if detected_tables == 1 { "" } else { "s" },
                included_pages.len(),
                if included_pages.len() == 1 { "" } else { "s" }
            ));
    Ok(PdfParseResult {
        sheets,
        page_count,
        included_pages: included_pages.into_iter().collect(),
        detected_tables,
    })
}

fn parse_page_selection(selection: Option<&str>, page_count: u32) -> Result<Vec<u32>> {
    let selection = selection.unwrap_or_default().trim();
    if selection.is_empty() {
        return Ok((1..=page_count).collect());
    }
    let mut pages = BTreeSet::new();
    for part in selection.split(',') {
        let part = part.trim();
        ensure!(
            !part.is_empty(),
            "Page selection contains an empty item. Use a format such as 1-3, 7, 10."
        );
        let bounds = part.split('-').map(str::trim).collect::<Vec<_>>();
        ensure!(
            bounds.len() <= 2 && bounds.iter().all(|value| !value.is_empty()),
            "Invalid page selection '{part}'. Use a format such as 1-3, 7, 10."
        );
        let start = bounds[0]
            .parse::<u32>()
            .with_context(|| format!("Invalid page number '{}'.", bounds[0]))?;
        let end = if bounds.len() == 2 {
            bounds[1]
                .parse::<u32>()
                .with_context(|| format!("Invalid page number '{}'.", bounds[1]))?
        } else {
            start
        };
        ensure!(
            start > 0 && start <= end && end <= page_count,
            "Page range '{part}' is outside this PDF's 1-{page_count} page range."
        );
        pages.extend(start..=end);
    }
    ensure!(!pages.is_empty(), "Choose at least one page.");
    Ok(pages.into_iter().collect())
}

struct PdfTableBlock {
    start: usize,
    end: usize,
    rows: Vec<Vec<String>>,
}

fn detect_table_blocks(lines: &[String]) -> Vec<PdfTableBlock> {
    fn flush(
        current: &mut Vec<Vec<String>>,
        start: usize,
        end: usize,
        blocks: &mut Vec<PdfTableBlock>,
    ) {
        let consistent = current.iter().any(|row| {
            let columns = row.len();
            columns >= 2
                && current
                    .iter()
                    .filter(|other| other.len() == columns)
                    .count()
                    >= 2
        });
        if current.len() >= 2 && consistent {
            blocks.push(PdfTableBlock {
                start,
                end,
                rows: std::mem::take(current),
            });
        } else {
            current.clear();
        }
    }

    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut start = 0;
    for (index, line) in lines.iter().enumerate() {
        if let Some(columns) = split_table_columns(line) {
            if current.is_empty() {
                start = index;
            }
            current.push(columns);
        } else {
            flush(&mut current, start, index, &mut blocks);
        }
    }
    flush(&mut current, start, lines.len(), &mut blocks);
    blocks
}

fn split_table_columns(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim_matches(|character| character == ' ' || character == '\r');
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('\t') {
        let columns = trimmed
            .split('\t')
            .map(str::trim)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        return (columns.len() >= 2 && columns.iter().any(|value| !value.is_empty()))
            .then_some(columns);
    }
    if trimmed.matches('|').count() >= 1 {
        let row = if trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.len() >= 2 {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };
        let columns = row
            .split('|')
            .map(str::trim)
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if columns.len() >= 2 && columns.iter().any(|value| !value.is_empty()) {
            return Some(columns);
        }
    }

    let mut columns = Vec::new();
    let mut current = String::new();
    let mut whitespace = 0usize;
    for character in trimmed.chars() {
        if character.is_whitespace() {
            whitespace += 1;
            continue;
        }
        if whitespace >= 2 && !current.trim().is_empty() {
            columns.push(current.trim().to_owned());
            current.clear();
        } else if whitespace == 1 && !current.is_empty() {
            current.push(' ');
        }
        whitespace = 0;
        current.push(character);
    }
    if !current.trim().is_empty() {
        columns.push(current.trim().to_owned());
    }
    (columns.len() >= 2).then_some(columns)
}
