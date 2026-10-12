//! Streaming CSV reading and writing.

pub mod reader;
pub mod table;
pub mod writer;

pub use reader::{CsvError, CsvReader, Record};
pub use table::{DocumentTables, TableNotes, TableRows, emit_table};
pub use writer::CsvWriter;

/// The delimiter a comma-separated file is really written with, from its
/// first records (`head`, the file's start; `whole` when it is all of it):
/// the delimiter every record of the first ten holds the same number of,
/// outside quotes, a comma when it does. A semicolon export from a European
/// spreadsheet, its decimals written with commas, reads as semicolons; a
/// file no delimiter fits reads as commas.
pub fn sniff_delimiter(head: &[u8], whole: bool) -> u8 {
    const CANDIDATES: [u8; 4] = *b",;\t|";
    const RECORDS: usize = 10;
    let mut records: Vec<[usize; 4]> = Vec::new();
    let mut counts = [0usize; 4];
    let mut quoted = false;
    let mut pending = false;
    for &byte in head {
        match byte {
            b'"' => quoted = !quoted,
            b'\n' if !quoted => {
                records.push(counts);
                counts = [0; 4];
                pending = false;
                if records.len() == RECORDS {
                    break;
                }
                continue;
            }
            _ if quoted => {}
            _ => {
                if let Some(index) = CANDIDATES.iter().position(|candidate| *candidate == byte) {
                    counts[index] += 1;
                }
            }
        }
        pending = true;
    }
    // The file's last record, without its line break.
    if whole && pending && !quoted && records.len() < RECORDS {
        records.push(counts);
    }
    let Some(first) = records.first() else {
        return b',';
    };
    CANDIDATES
        .iter()
        .enumerate()
        .find(|(index, _)| {
            first[*index] > 0 && records.iter().all(|record| record[*index] == first[*index])
        })
        .map_or(b',', |(_, candidate)| *candidate)
}

#[cfg(test)]
mod sniff_tests {
    use super::sniff_delimiter;

    #[test]
    fn delimiters_are_read_from_the_first_records() {
        assert_eq!(sniff_delimiter(b"a;b;c\n1;2;3\n", true), b';');
        assert_eq!(sniff_delimiter(b"a|b\n1|2", true), b'|');
        assert_eq!(sniff_delimiter(b"a\tb\n1\t2\n", true), b'\t');
        assert_eq!(sniff_delimiter(b"a,b\n1,2\n", true), b',');
        // Decimal commas in a semicolon export do not count against it.
        assert_eq!(sniff_delimiter(b"a;b;c\n1,5;2;x\n3;4,25;y\n", true), b';');
        // Counts that disagree keep the comma.
        assert_eq!(sniff_delimiter(b"note;x\nplain\n", true), b',');
        assert_eq!(sniff_delimiter(b"a,b;c\n1,2;3\n", true), b',');
        // Quoted delimiters do not count.
        assert_eq!(sniff_delimiter(b"\"a,b\";c\n\"1,2\";3\n", true), b';');
        assert_eq!(sniff_delimiter(b"one column\n", true), b',');
        assert_eq!(sniff_delimiter(b"", true), b',');
        // A cut-off head leaves its partial record out.
        assert_eq!(sniff_delimiter(b"a;b\n1;2\n3", false), b';');
        assert_eq!(sniff_delimiter(b"a;b;c", false), b',');
    }
}
