//! PDF to XLSX: each page's text put back into rows and columns, from where MuPDF says
//! every character sits (`mutool draw -F stext`), and written as an Excel workbook with
//! one sheet per page. The workbook is written here, a few small XML files in a ZIP,
//! so no spreadsheet program is needed.
//!
//! Rows are text at the same height. Columns are found from the rows with more than one
//! piece of text: the strips of the page those pieces cover, merged where they overlap,
//! so right-aligned numbers and left-aligned headers above them share a column. A row
//! with one piece (a title, a paragraph) goes to the column it overlaps, or the first.

use std::io::{self, Write};
use std::path::Path;

/// One character where MuPDF placed it, in points from the page's top left.
#[derive(Clone, Debug)]
struct Glyph {
    c: char,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

/// A run of characters on one line, with a big gap before and after: one cell.
#[derive(Clone, Debug, PartialEq)]
struct Piece {
    text: String,
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Piece {
    fn middle(&self) -> f64 {
        (self.y0 + self.y1) / 2.0
    }

    fn height(&self) -> f64 {
        (self.y1 - self.y0).max(0.1)
    }
}

/// One cell of a sheet.
#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Empty,
    Text(String),
    /// A number as written in the PDF, without thousands commas ("1234.56").
    Number(String),
}

/// One page, as rows of cells.
pub type Table = Vec<Vec<Cell>>;

/// At most this many columns on a page; a page with more is taken as plain text.
const MAX_COLUMNS: usize = 200;

/// The pages in MuPDF's structured text (`mutool draw -F stext`), as pieces of text.
fn read_pages(xml: &str) -> Vec<Vec<Piece>> {
    let mut pages: Vec<Vec<Piece>> = Vec::new();
    let mut line: Vec<Glyph> = Vec::new();
    let mut size = 0.0;
    let mut rest = xml;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        let Some(close) = tag_end(rest) else {
            break;
        };
        let tag = &rest[..close];
        rest = &rest[close + 1..];
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or_default();
        match name {
            "page" => pages.push(Vec::new()),
            "line" => line.clear(),
            "font" => size = attr(tag, "size").and_then(|s| s.parse().ok()).unwrap_or(0.0),
            "char" => {
                let c = attr(tag, "c").and_then(|c| c.chars().next());
                let quad: Vec<f64> = attr(tag, "quad").map(|q| q.split_whitespace().filter_map(|n| n.parse().ok()).collect()).unwrap_or_default();
                if let (Some(c), [ulx, uly, urx, ury, llx, lly, lrx, lry]) = (c, quad.as_slice()) {
                    line.push(Glyph {
                        c,
                        x0: ulx.min(*llx),
                        x1: urx.max(*lrx),
                        y0: uly.min(*ury),
                        y1: lly.max(*lry),
                    });
                }
            }
            "" if tag.starts_with("/line") => {
                if let Some(page) = pages.last_mut() {
                    page.extend(pieces(&line, size));
                }
                line.clear();
            }
            _ => {}
        }
    }
    pages
}

/// Where a tag ends: the first ">" outside quotes.
fn tag_end(text: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in text.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (q, Some(open)) if q == open => quote = None,
            ('>', None) => return Some(i),
            _ => {}
        }
    }
    None
}

/// An attribute's value, with XML's escapes undone.
fn attr(tag: &str, name: &str) -> Option<String> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().last();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.starts_with('=') {
            let after = &after[1..];
            let quote = after.chars().next()?;
            let value = &after[1..];
            let end = value.find(quote)?;
            return Some(unescape(&value[..end]));
        }
        rest = after;
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else {
            break;
        };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| entity.strip_prefix('#').map(str::parse::<u32>))
                .and_then(Result::ok)
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A line cut into pieces wherever the gap between two characters is about a letter
/// wide or more (a word space is a third of that).
fn pieces(line: &[Glyph], size: f64) -> Vec<Piece> {
    let mut out: Vec<Piece> = Vec::new();
    let mut current: Option<Piece> = None;
    let mut last_end: Option<f64> = None;
    for g in line {
        if g.c.is_whitespace() || g.c.is_control() {
            continue;
        }
        let em = if size > 0.0 { size } else { g.y1 - g.y0 };
        let gap = last_end.map(|end| g.x0 - end);
        match (&mut current, gap) {
            (Some(piece), Some(gap)) if gap < 0.9 * em => {
                // A gap wider than a fifth of a letter is a word space.
                if gap > 0.2 * em {
                    piece.text.push(' ');
                }
                piece.text.push(g.c);
                piece.x1 = piece.x1.max(g.x1);
                piece.y0 = piece.y0.min(g.y0);
                piece.y1 = piece.y1.max(g.y1);
            }
            _ => {
                if let Some(done) = current.take() {
                    out.push(done);
                }
                current = Some(Piece { text: g.c.to_string(), x0: g.x0, x1: g.x1, y0: g.y0, y1: g.y1 });
            }
        }
        last_end = Some(g.x1);
    }
    out.extend(current);
    out
}

/// Pieces at the same height, each row from left to right, rows from the top.
fn rows(mut pieces: Vec<Piece>) -> Vec<Vec<Piece>> {
    pieces.sort_by(|a, b| a.middle().total_cmp(&b.middle()));
    let mut rows: Vec<Vec<Piece>> = Vec::new();
    for piece in pieces {
        let joins = rows.last().is_some_and(|row| {
            let (middle, height) = row_band(row);
            (piece.middle() - middle).abs() < 0.45 * height.max(piece.height())
        });
        match rows.last_mut() {
            Some(row) if joins => row.push(piece),
            _ => rows.push(vec![piece]),
        }
    }
    for row in &mut rows {
        row.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    }
    rows
}

fn row_band(row: &[Piece]) -> (f64, f64) {
    let n = row.len() as f64;
    (row.iter().map(Piece::middle).sum::<f64>() / n, row.iter().map(Piece::height).sum::<f64>() / n)
}

/// The columns: strips of the page covered by the pieces of rows with more than one,
/// merged where they overlap, from left to right.
fn columns(rows: &[Vec<Piece>]) -> Vec<(f64, f64)> {
    let mut strips: Vec<(f64, f64)> = rows.iter().filter(|r| r.len() > 1).flatten().map(|p| (p.x0, p.x1)).collect();
    strips.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (x0, x1) in strips {
        match merged.last_mut() {
            Some(last) if x0 <= last.1 + 0.5 => last.1 = last.1.max(x1),
            _ => merged.push((x0, x1)),
        }
    }
    merged
}

/// The column a piece goes in: the one it overlaps most. A lone piece on its row (a
/// title, a paragraph, a single value) only when at least half of it lies in that
/// column, otherwise the first: a title that just reaches into the last column stays
/// at the left.
fn column_of(piece: &Piece, columns: &[(f64, f64)], lone: bool) -> usize {
    let overlap = |c: &(f64, f64)| piece.x1.min(c.1) - piece.x0.max(c.0);
    let best = columns.iter().enumerate().max_by(|a, b| overlap(a.1).total_cmp(&overlap(b.1)));
    let enough = if lone { 0.5 * (piece.x1 - piece.x0) } else { 0.0 };
    match best {
        Some((i, c)) if overlap(c) > enough => i,
        // Between two columns: the nearest one, by its left edge.
        _ if !lone => columns
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1 .0 - piece.x0).abs().total_cmp(&(b.1 .0 - piece.x0).abs()))
            .map_or(0, |(i, _)| i),
        _ => 0,
    }
}

/// One page's pieces as a table.
fn table(pieces: Vec<Piece>) -> Table {
    let rows = rows(pieces);
    let mut columns = columns(&rows);
    if columns.len() > MAX_COLUMNS {
        columns.clear();
    }
    rows.iter()
        .map(|row| {
            let width = columns.len().max(1);
            let mut cells: Vec<Vec<&str>> = vec![Vec::new(); width];
            for piece in row {
                let at = if columns.is_empty() { 0 } else { column_of(piece, &columns, row.len() == 1) };
                cells[at].push(&piece.text);
            }
            let mut cells: Vec<Cell> = cells.into_iter().map(|parts| cell(&parts.join(" "))).collect();
            while cells.last() == Some(&Cell::Empty) {
                cells.pop();
            }
            cells
        })
        .collect()
}

/// A cell's text as a number when it is plainly one: "1200", "-3", "4.50",
/// "1,234.56". Leading zeros ("007"), codes and long digit runs stay text, as do
/// numbers with a decimal comma, which can't be told from thousands.
fn cell(text: &str) -> Cell {
    let text = text.trim();
    if text.is_empty() {
        return Cell::Empty;
    }
    match number(text) {
        Some(n) => Cell::Number(n),
        None => Cell::Text(text.to_string()),
    }
}

fn number(text: &str) -> Option<String> {
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (digits, None),
    };
    if let Some(f) = fraction {
        if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let groups: Vec<&str> = whole.split(',').collect();
    let first = groups.first()?;
    let grouped = groups.len() > 1;
    let ok = !first.is_empty()
        && first.bytes().all(|b| b.is_ascii_digit())
        && (!grouped || first.len() <= 3)
        && groups[1..].iter().all(|g| g.len() == 3 && g.bytes().all(|b| b.is_ascii_digit()))
        // "0", "0.5" but not "007".
        && !(first.len() > 1 && first.starts_with('0') && !grouped);
    let plain: String = groups.concat();
    // Beyond 15 digits Excel would round it: an account number, kept as text.
    if !ok || plain.len() + fraction.map_or(0, str::len) > 15 {
        return None;
    }
    Some(match fraction {
        Some(f) => format!("{sign}{plain}.{f}"),
        None => format!("{sign}{plain}"),
    })
}

/// Every page of MuPDF's structured text as a table.
pub fn tables(xml: &str) -> Vec<Table> {
    read_pages(xml).into_iter().map(table).collect()
}

/// Writes `sheets` (name, rows) as an XLSX workbook.
pub fn write_xlsx(path: &Path, sheets: &[(String, Table)]) -> io::Result<()> {
    let mut parts: Vec<(String, Vec<u8>)> = Vec::new();
    let mut types = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    let mut book = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>"#,
    );
    let mut rels = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
    );
    for (i, (name, rows)) in sheets.iter().enumerate() {
        let n = i + 1;
        types.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        book.push_str(&format!(r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#, escape(&sheet_name(name, n))));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
        parts.push((format!("xl/worksheets/sheet{n}.xml"), worksheet(rows).into_bytes()));
    }
    types.push_str("</Types>");
    book.push_str("</sheets></workbook>");
    rels.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#,
        sheets.len() + 1
    ));
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("[Content_Types].xml".into(), types.into_bytes()),
        (
            "_rels/.rels".into(),
            br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
                .to_vec(),
        ),
        ("xl/workbook.xml".into(), book.into_bytes()),
        ("xl/_rels/workbook.xml.rels".into(), rels.into_bytes()),
        ("xl/styles.xml".into(), STYLES.as_bytes().to_vec()),
    ];
    files.extend(parts);
    let file = std::fs::File::create(path)?;
    let mut out = io::BufWriter::new(file);
    write_zip(&mut out, &files)?;
    out.flush()
}

/// The least a workbook's styles can be: one font, the two fills Excel expects, one
/// border and the normal cell style.
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#;

/// A sheet name Excel takes: at most 31 characters, none of []:*?/\, never empty.
fn sheet_name(name: &str, n: usize) -> String {
    let clean: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
    if clean.trim().is_empty() {
        format!("Sheet {n}")
    } else {
        clean
    }
}

/// "A", "B", ... "Z", "AA", ...
fn column_name(mut n: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

fn worksheet(rows: &Table) -> String {
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, row) in rows.iter().enumerate() {
        let r = r + 1;
        xml.push_str(&format!(r#"<row r="{r}">"#));
        for (c, cell) in row.iter().enumerate() {
            let at = format!("{}{r}", column_name(c));
            match cell {
                Cell::Empty => {}
                Cell::Number(n) => xml.push_str(&format!(r#"<c r="{at}"><v>{n}</v></c>"#)),
                Cell::Text(t) => xml.push_str(&format!(r#"<c r="{at}" t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#, escape(t))),
            }
        }
        xml.push_str("</row>");
    }
    xml.push_str("</sheetData></worksheet>");
    xml
}

/// Text for XML: the five special characters escaped, characters XML can't hold left out.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || matches!(c as u32, 0xFFFE | 0xFFFF) => {}
            c => out.push(c),
        }
    }
    out
}

/// A ZIP file of `files` (name, bytes), each compressed with deflate.
fn write_zip(out: &mut impl Write, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    use flate2::write::DeflateEncoder;
    use flate2::{Compression, Crc};
    // 1980-01-01 00:00, the earliest time ZIP can say.
    const TIME: u16 = 0;
    const DATE: u16 = (1 << 5) | 1;
    let mut offset: u32 = 0;
    let mut central = Vec::new();
    let too_big = || io::Error::other("the workbook is too large for a ZIP file");
    for (name, data) in files {
        let mut crc = Crc::new();
        crc.update(data);
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(data)?;
        let packed = encoder.finish()?;
        let (size, packed_size) = (u32::try_from(data.len()).map_err(|_| too_big())?, u32::try_from(packed.len()).map_err(|_| too_big())?);
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        let mut local = Vec::new();
        local.extend(0x0403_4b50u32.to_le_bytes());
        local.extend(20u16.to_le_bytes()); // version needed: 2.0
        local.extend(0u16.to_le_bytes()); // flags
        local.extend(8u16.to_le_bytes()); // deflate
        local.extend(TIME.to_le_bytes());
        local.extend(DATE.to_le_bytes());
        local.extend(crc.sum().to_le_bytes());
        local.extend(packed_size.to_le_bytes());
        local.extend(size.to_le_bytes());
        local.extend(name_len.to_le_bytes());
        local.extend(0u16.to_le_bytes()); // no extra field
        local.extend(name.as_bytes());
        out.write_all(&local)?;
        out.write_all(&packed)?;

        central.extend(0x0201_4b50u32.to_le_bytes());
        central.extend(20u16.to_le_bytes()); // made by: 2.0
        central.extend(20u16.to_le_bytes());
        central.extend(0u16.to_le_bytes());
        central.extend(8u16.to_le_bytes());
        central.extend(TIME.to_le_bytes());
        central.extend(DATE.to_le_bytes());
        central.extend(crc.sum().to_le_bytes());
        central.extend(packed_size.to_le_bytes());
        central.extend(size.to_le_bytes());
        central.extend(name_len.to_le_bytes());
        central.extend([0u8; 12]); // extra, comment, disk, internal and external attributes
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());

        let written = u32::try_from(local.len() + packed.len()).map_err(|_| too_big())?;
        offset = offset.checked_add(written).ok_or_else(too_big)?;
    }
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let central_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.write_all(&central)?;
    let mut end = Vec::new();
    end.extend(0x0605_4b50u32.to_le_bytes());
    end.extend([0u8; 4]); // this disk, the disk the directory starts on
    end.extend(count.to_le_bytes());
    end.extend(count.to_le_bytes());
    end.extend(central_size.to_le_bytes());
    end.extend(offset.to_le_bytes());
    end.extend(0u16.to_le_bytes()); // no comment
    out.write_all(&end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A line of MuPDF's structured text: each character 5 points wide from `x`.
    fn line(x: f64, y: f64, text: &str) -> String {
        let mut xml = format!(r#"<line bbox="0 0 0 0" wmode="0" dir="1 0" text="{text}"><font name="F" size="10">"#);
        for (i, c) in text.chars().enumerate() {
            let x0 = x + 5.0 * i as f64;
            let x1 = x0 + 5.0;
            let c = match c {
                '&' => "&amp;".to_string(),
                '<' => "&lt;".to_string(),
                '"' => "&quot;".to_string(),
                c => c.to_string(),
            };
            xml.push_str(&format!(r#"<char c="{c}" quad="{x0} {y} {x1} {y} {x0} {} {x1} {}" x="{x0}" y="{}"/>"#, y + 10.0, y + 10.0, y + 9.0));
            xml.push('\n');
        }
        xml.push_str("</font></line>\n");
        xml
    }

    fn page(lines: &[String]) -> String {
        format!(r#"<page id="page1" width="600" height="800"><block bbox="0 0 0 0">{}</block></page>"#, lines.concat())
    }

    fn text(t: &str) -> Cell {
        Cell::Text(t.into())
    }

    fn num(n: &str) -> Cell {
        Cell::Number(n.into())
    }

    #[test]
    fn a_table_comes_back_in_rows_and_columns() {
        let xml = format!(
            "<document>{}</document>",
            page(&[
                line(250.0, 20.0, "Sales report"),
                line(50.0, 60.0, "Product"),
                line(150.0, 60.0, "Units"),
                line(220.0, 60.0, "Price"),
                line(50.0, 75.0, "Widget A"),
                // Right-aligned under "Units": overlaps it, so the same column.
                line(160.0, 75.0, "1200"),
                line(225.0, 75.0, "4.50"),
                line(50.0, 90.0, "Gadget & Co"),
                line(170.0, 90.0, "-3"),
                line(215.0, 90.0, "1,234.56"),
                // Only a price: it stays under Price.
                line(225.0, 105.0, "9.99"),
            ])
        );
        let pages = tables(&xml);
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0],
            vec![
                vec![text("Sales report")],
                vec![text("Product"), text("Units"), text("Price")],
                vec![text("Widget A"), num("1200"), num("4.50")],
                vec![text("Gadget & Co"), num("-3"), num("1234.56")],
                vec![Cell::Empty, Cell::Empty, num("9.99")],
            ]
        );
    }

    #[test]
    fn one_line_with_wide_gaps_becomes_several_cells() {
        // Columns joined into one MuPDF line, with gaps of two letters between them.
        let mut xml = String::from(r#"<page><line><font size="10">"#);
        let mut x = 10.0;
        for (word, gap) in [("Name", 10.0), ("Age", 10.0), ("City", 0.0)] {
            for c in word.chars() {
                xml.push_str(&format!(r#"<char c="{c}" quad="{x} 0 {} 0 {x} 10 {} 10"/>"#, x + 5.0, x + 5.0));
                x += 5.0;
            }
            // A word space is a narrower gap: still one cell.
            x += gap;
        }
        xml.push_str("</font></line></page>");
        let pages = read_pages(&xml);
        let words: Vec<&str> = pages[0].iter().map(|p| p.text.as_str()).collect();
        assert_eq!(words, ["Name", "Age", "City"]);
    }

    #[test]
    fn numbers_are_only_what_is_plainly_a_number() {
        for (t, n) in [("1200", Some("1200")), ("-3", Some("-3")), ("4.50", Some("4.50")), ("1,234,567.8", Some("1234567.8")), ("0.5", Some("0.5")), ("0", Some("0"))] {
            assert_eq!(number(t).as_deref(), n, "{t}");
        }
        for t in ["007", "1,23", "12,3456", "1.234,56", "12.", "3 kg", "€5", "1-2", "", "1234567890123456", "+5", "1e5"] {
            assert_eq!(number(t), None, "{t}");
        }
    }

    #[test]
    fn attributes_and_escapes_are_read() {
        let tag = r#"char c="&lt;" quad="1 2 3 4 5 6 7 8" x="1""#;
        assert_eq!(attr(tag, "c").as_deref(), Some("<"));
        assert_eq!(attr(tag, "x").as_deref(), Some("1"));
        assert_eq!(attr(r#"char cx="9" c="a""#, "c").as_deref(), Some("a"));
        assert_eq!(unescape("a &amp; b &#233; &#x41; &bogus; &"), "a & b é A &bogus; &");
        // The ">" inside the quotes doesn't end the tag.
        assert_eq!(tag_end(r#"char c=">" x="1"/>rest"#), Some(17));
    }

    #[test]
    fn names_and_columns_are_what_excel_takes() {
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
        assert_eq!(sheet_name("Page 1", 1), "Page 1");
        assert_eq!(sheet_name("[a]:b*c?d/e\\f", 2), "abcdef");
        assert_eq!(sheet_name("???", 3), "Sheet 3");
        assert_eq!(escape("a<b & \"c\"\u{1}"), "a&lt;b &amp; &quot;c&quot;");
    }

    #[test]
    fn the_workbook_is_a_zip_with_every_part() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("book.xlsx");
        let sheets = vec![("Page 1".to_string(), vec![vec![text("Name"), num("12.5")]]), ("Page 2".to_string(), Vec::new())];
        write_xlsx(&path, &sheets).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"PK\x03\x04"));
        // The end record says seven files: the five fixed parts and two sheets.
        let end = bytes.windows(4).rposition(|w| w == b"PK\x05\x06").unwrap();
        assert_eq!(u16::from_le_bytes([bytes[end + 10], bytes[end + 11]]), 7);
        // Read back with the system's own unzip where there is one.
        if let Ok(out) = std::process::Command::new("unzip").arg("-p").arg(&path).arg("xl/worksheets/sheet1.xml").output() {
            if out.status.success() {
                let sheet = String::from_utf8(out.stdout).unwrap();
                assert!(sheet.contains(r#"<c r="A1" t="inlineStr"><is><t xml:space="preserve">Name</t></is></c><c r="B1"><v>12.5</v></c>"#), "{sheet}");
                let test = std::process::Command::new("unzip").arg("-tq").arg(&path).output().unwrap();
                assert!(test.status.success(), "{}", String::from_utf8_lossy(&test.stdout));
            }
        }
    }
}
