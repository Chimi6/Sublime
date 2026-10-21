//! The one list of converters. Adding a converter is one line here plus one
//! `pub mod` line in `src/converters/mod.rs`.

use std::sync::OnceLock;

use crate::converter::Entry;
use crate::converters::csv_to_json;
use crate::converters::csv_to_xlsx;
use crate::converters::docx_to_html::DocxToHtml;
use crate::converters::docx_to_markdown::DocxToMarkdown;
use crate::converters::docx_to_pages::DocxToPages;
use crate::converters::docx_to_text::DocxToText;
use crate::converters::epub;
use crate::converters::html_to_docx::HtmlToDocx;
use crate::converters::html_to_markdown::HtmlToMarkdown;
use crate::converters::html_to_pages::HtmlToPages;
use crate::converters::html_to_text::HtmlToText;
use crate::converters::hub;
use crate::converters::image;
use crate::converters::json_to_csv;
use crate::converters::json_to_pages::JsonToPages;
use crate::converters::json_to_toml::JsonToToml;
use crate::converters::json_to_xml::JsonToXml;
use crate::converters::json_to_yaml::JsonToYaml;
use crate::converters::markdown_json_to_markdown::MarkdownJsonToMarkdown;
use crate::converters::markdown_to_docx::MarkdownToDocx;
use crate::converters::markdown_to_html::MarkdownToHtml;
use crate::converters::markdown_to_json::MarkdownToJson;
use crate::converters::markdown_to_pages::MarkdownToPages;
use crate::converters::markdown_to_text::MarkdownToText;
use crate::converters::n64;
use crate::converters::numbers;
use crate::converters::ods;
use crate::converters::odt;
use crate::converters::pages_to_docx::PagesToDocx;
use crate::converters::pages_to_html::PagesToHtml;
use crate::converters::pages_to_json::PagesToJson;
use crate::converters::pages_to_markdown::PagesToMarkdown;
use crate::converters::pages_to_text::PagesToText;
use crate::converters::pdf_to_document;
use crate::converters::pdf_to_text::PdfToText;
use crate::converters::rows;
use crate::converters::rows_document;
use crate::converters::rtf;
use crate::converters::text_to_docx::TextToDocx;
use crate::converters::text_to_html::TextToHtml;
use crate::converters::text_to_markdown::TextToMarkdown;
use crate::converters::text_to_pages::TextToPages;
use crate::converters::to_numbers;
use crate::converters::to_pdf;
use crate::converters::toml_to_json::TomlToJson;
use crate::converters::xlsx_to_csv;
use crate::converters::xml_to_json::XmlToJson;
use crate::converters::yaml_to_json::YamlToJson;
use crate::format::Format;

static CONVERTERS: [Entry; 140] = [
    csv_to_json::CSV_TO_JSON.entry(),
    json_to_csv::JSON_TO_CSV.entry(),
    MarkdownToHtml.entry(),
    MarkdownToText.entry(),
    MarkdownToJson.entry(),
    MarkdownJsonToMarkdown.entry(),
    PagesToJson.entry(),
    JsonToPages.entry(),
    PagesToDocx.entry(),
    PagesToMarkdown.entry(),
    PagesToHtml.entry(),
    PagesToText.entry(),
    DocxToMarkdown.entry(),
    DocxToHtml.entry(),
    DocxToText.entry(),
    MarkdownToDocx.entry(),
    MarkdownToPages.entry(),
    TextToPages.entry(),
    HtmlToPages.entry(),
    DocxToPages.entry(),
    HtmlToMarkdown.entry(),
    HtmlToText.entry(),
    HtmlToDocx.entry(),
    TextToMarkdown.entry(),
    TextToHtml.entry(),
    TextToDocx.entry(),
    PdfToText.entry(),
    pdf_to_document::PDF_TO_MARKDOWN.entry(),
    pdf_to_document::PDF_TO_HTML.entry(),
    pdf_to_document::PDF_TO_DOCX.entry(),
    to_pdf::MARKDOWN_TO_PDF.entry(),
    to_pdf::HTML_TO_PDF.entry(),
    to_pdf::TEXT_TO_PDF.entry(),
    to_pdf::DOCX_TO_PDF.entry(),
    to_pdf::PAGES_TO_PDF.entry(),
    to_pdf::RTF_TO_PDF.entry(),
    epub::EPUB_TO_DOCX.entry(),
    epub::EPUB_TO_PAGES.entry(),
    epub::EPUB_TO_RTF.entry(),
    epub::EPUB_TO_ODT.entry(),
    epub::EPUB_TO_MARKDOWN.entry(),
    epub::EPUB_TO_HTML.entry(),
    epub::EPUB_TO_TEXT.entry(),
    to_pdf::EPUB_TO_PDF.entry(),
    epub::DOCX_TO_EPUB.entry(),
    epub::PAGES_TO_EPUB.entry(),
    epub::RTF_TO_EPUB.entry(),
    epub::ODT_TO_EPUB.entry(),
    epub::MARKDOWN_TO_EPUB.entry(),
    epub::HTML_TO_EPUB.entry(),
    epub::TEXT_TO_EPUB.entry(),
    pdf_to_document::PDF_TO_EPUB.entry(),
    odt::ODT_TO_DOCX.entry(),
    odt::ODT_TO_PAGES.entry(),
    odt::ODT_TO_RTF.entry(),
    odt::ODT_TO_MARKDOWN.entry(),
    odt::ODT_TO_HTML.entry(),
    odt::ODT_TO_TEXT.entry(),
    to_pdf::ODT_TO_PDF.entry(),
    odt::DOCX_TO_ODT.entry(),
    odt::PAGES_TO_ODT.entry(),
    odt::RTF_TO_ODT.entry(),
    odt::MARKDOWN_TO_ODT.entry(),
    odt::HTML_TO_ODT.entry(),
    odt::TEXT_TO_ODT.entry(),
    pdf_to_document::PDF_TO_ODT.entry(),
    rtf::RTF_TO_DOCX.entry(),
    rtf::RTF_TO_PAGES.entry(),
    rtf::RTF_TO_MARKDOWN.entry(),
    rtf::RTF_TO_HTML.entry(),
    rtf::RTF_TO_TEXT.entry(),
    rtf::DOCX_TO_RTF.entry(),
    rtf::PAGES_TO_RTF.entry(),
    rtf::MARKDOWN_TO_RTF.entry(),
    rtf::HTML_TO_RTF.entry(),
    rtf::TEXT_TO_RTF.entry(),
    TomlToJson.entry(),
    JsonToToml.entry(),
    YamlToJson.entry(),
    JsonToYaml.entry(),
    XmlToJson.entry(),
    JsonToXml.entry(),
    hub::TOML_TO_YAML.entry(),
    hub::YAML_TO_TOML.entry(),
    hub::TOML_TO_XML.entry(),
    hub::XML_TO_TOML.entry(),
    hub::YAML_TO_XML.entry(),
    hub::XML_TO_YAML.entry(),
    csv_to_json::TSV_TO_JSON.entry(),
    csv_to_json::CSV_TO_JSONL.entry(),
    csv_to_json::TSV_TO_JSONL.entry(),
    json_to_csv::JSON_TO_TSV.entry(),
    json_to_csv::JSONL_TO_CSV.entry(),
    json_to_csv::JSONL_TO_TSV.entry(),
    rows::CSV_TO_TSV.entry(),
    rows::TSV_TO_CSV.entry(),
    rows::JsonlToJson.entry(),
    rows::JsonToJsonl.entry(),
    xlsx_to_csv::XLSX_TO_CSV.entry(),
    xlsx_to_csv::XLSX_TO_TSV.entry(),
    xlsx_to_csv::XLSX_TO_JSON.entry(),
    xlsx_to_csv::XLSX_TO_MARKDOWN.entry(),
    numbers::NUMBERS_TO_CSV.entry(),
    numbers::NUMBERS_TO_TSV.entry(),
    numbers::NUMBERS_TO_JSON.entry(),
    numbers::NUMBERS_TO_MARKDOWN.entry(),
    numbers::NUMBERS_TO_XLSX.entry(),
    ods::ODS_TO_CSV.entry(),
    ods::ODS_TO_TSV.entry(),
    ods::ODS_TO_JSON.entry(),
    ods::ODS_TO_MARKDOWN.entry(),
    ods::ODS_TO_XLSX.entry(),
    ods::ODS_TO_NUMBERS.entry(),
    ods::CSV_TO_ODS.entry(),
    ods::TSV_TO_ODS.entry(),
    ods::XLSX_TO_ODS.entry(),
    ods::JSON_TO_ODS.entry(),
    ods::MARKDOWN_TO_ODS.entry(),
    ods::NUMBERS_TO_ODS.entry(),
    to_numbers::CSV_TO_NUMBERS.entry(),
    to_numbers::TSV_TO_NUMBERS.entry(),
    to_numbers::XLSX_TO_NUMBERS.entry(),
    to_numbers::JSON_TO_NUMBERS.entry(),
    to_numbers::MARKDOWN_TO_NUMBERS.entry(),
    rows_document::MARKDOWN_TO_XLSX.entry(),
    json_to_csv::JSON_TO_XLSX.entry(),
    rows_document::MARKDOWN_TABLES_TO_JSON.entry(),
    rows_document::JSON_TO_MARKDOWN.entry(),
    csv_to_xlsx::CSV_TO_XLSX.entry(),
    csv_to_xlsx::TSV_TO_XLSX.entry(),
    rows_document::CSV_TO_MARKDOWN.entry(),
    rows_document::TSV_TO_MARKDOWN.entry(),
    rows_document::MARKDOWN_TO_CSV.entry(),
    rows_document::MARKDOWN_TO_TSV.entry(),
    n64::Z64_TO_V64.entry(),
    n64::Z64_TO_N64.entry(),
    n64::V64_TO_Z64.entry(),
    n64::V64_TO_N64.entry(),
    n64::N64_TO_Z64.entry(),
    n64::N64_TO_V64.entry(),
];

/// The listed converters and the generated image pairs, gathered once:
/// what the planner reads. Nothing here runs a converter's code.
pub fn all_converters() -> &'static [Entry] {
    static ALL: OnceLock<Vec<Entry>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all: Vec<Entry> = CONVERTERS.to_vec();
        for pair in image::pairs() {
            all.push(pair.entry());
        }
        all
    })
}

/// The converters a conversion between these formats (and through `via`)
/// can use. An image pair joins two image formats, and the only listed
/// converters into or out of one are PDF's, so a path between formats that
/// are not images never takes a pair (it would leave PDF and come back):
/// building the pairs, about 400 KB of a conversion's memory, is left to
/// conversions with an image at one end.
pub fn converters_for(
    from: &'static Format,
    to: &'static Format,
    via: Option<&'static Format>,
) -> &'static [Entry] {
    let touches_images = [Some(from), Some(to), via]
        .into_iter()
        .flatten()
        .any(image::is_image_format);
    if touches_images {
        all_converters()
    } else {
        &CONVERTERS
    }
}

/// Every format referenced by a registered converter, unique, sorted by id.
pub fn all_formats() -> Vec<&'static Format> {
    let mut formats: Vec<&'static Format> = Vec::new();
    for converter in &CONVERTERS {
        push_unique(&mut formats, converter.from);
        push_unique(&mut formats, converter.to);
    }
    // The image pairs join every codec's format, read from the codec table
    // rather than the pairs, which are not built for this.
    for (format, _, _) in image::codecs() {
        push_unique(&mut formats, format);
    }
    formats.sort_by(|left, right| left.id.cmp(right.id));
    formats
}

fn push_unique(formats: &mut Vec<&'static Format>, format: &'static Format) {
    let already_present = formats.iter().any(|existing| existing.id == format.id);
    if !already_present {
        formats.push(format);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// `converters_for` leaves the image pairs out only where no plan uses
    /// them: every pair of formats plans the same with and without them.
    #[test]
    fn plans_without_the_image_pairs_are_unchanged() {
        use crate::planner::{PlanOptions, plan};
        let formats = all_formats();
        for strict in [false, true] {
            let options = PlanOptions { strict, via: None };
            for from in &formats {
                for to in &formats {
                    if from == to {
                        continue;
                    }
                    let full = plan(all_converters(), from, to, &options);
                    let chosen = plan(converters_for(from, to, None), from, to, &options);
                    let names = |result: &Result<crate::planner::Plan, _>| match result {
                        Ok(plan) => format!("{plan:?}"),
                        Err(error) => format!("{error:?}"),
                    };
                    assert_eq!(names(&full), names(&chosen), "{} -> {}", from.id, to.id);
                }
            }
        }
    }

    #[test]
    fn every_format_is_an_end_of_some_converter() {
        let formats = all_formats();
        for format in &formats {
            let used = all_converters()
                .iter()
                .any(|converter| converter.from == *format || converter.to == *format);
            assert!(used, "{} has no converter", format.id);
        }
        for converter in all_converters() {
            assert!(formats.contains(&converter.from) && formats.contains(&converter.to));
        }
    }

    #[test]
    fn converter_names_are_unique() {
        let mut names = HashSet::new();
        for converter in all_converters() {
            let is_new = names.insert(converter.name);
            assert!(is_new, "duplicate converter name {}", converter.name);
        }
    }

    #[test]
    fn edges_are_unique() {
        let mut edges = HashSet::new();
        for converter in all_converters() {
            let edge = (converter.from.id, converter.to.id);
            let is_new = edges.insert(edge);
            assert!(is_new, "duplicate edge {:?}", edge);
        }
    }

    /// Formats selected only by id (`--to`, `--from`), because no file
    /// extension is theirs alone.
    const EXTENSIONLESS: [&str; 2] = ["markdown-json", "pages-json"];

    #[test]
    fn every_format_has_an_extension_unless_listed() {
        for format in all_formats() {
            let listed = EXTENSIONLESS.contains(&format.id);
            assert!(
                !format.extensions.is_empty() || listed,
                "{} has no extensions",
                format.id
            );
        }
    }

    #[test]
    fn formats_are_sorted_and_unique() {
        let formats = all_formats();
        let ids: Vec<&str> = formats.iter().map(|format| format.id).collect();
        assert_eq!(
            ids,
            vec![
                "bmp",
                "csv",
                "cur",
                "docx",
                "epub",
                "heic",
                "html",
                "ico",
                "jpeg",
                "json",
                "jsonl",
                "markdown",
                "markdown-json",
                "n64",
                "numbers",
                "ods",
                "odt",
                "pages",
                "pages-json",
                "pam",
                "pbm",
                "pdf",
                "pgm",
                "png",
                "ppm",
                "qoi",
                "rtf",
                "text",
                "tga",
                "tiff",
                "toml",
                "tsv",
                "v64",
                "webp",
                "xlsx",
                "xml",
                "yaml",
                "z64"
            ]
        );
    }

    #[test]
    fn format_ids_are_lowercase() {
        for format in all_formats() {
            let lowered = format.id.to_ascii_lowercase();
            assert_eq!(format.id, lowered, "{} must be lowercase", format.id);
        }
    }
}
