//! Delimited-line splitting (Plan 45): the RFC 4180 subset a log viewer needs
//! — quoted sections hide the delimiter and `""` is a literal quote. Pure.

/// Split one delimited line by `delim`. A double-quote section may contain
/// the delimiter and `""` (literal quote). A trailing `\r` (CRLF leftovers)
/// is trimmed from the final field. Empty fields are preserved.
pub fn split_line(line: &str, delim: char) -> Vec<String> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.as_str().starts_with('"') {
                    chars.next();
                    cur.push('"'); // escaped quote
                } else {
                    in_quotes = false; // closing quote; the next char (often the
                                       // delimiter) is handled by the main loop
                }
            } else {
                cur.push(c);
            }
            continue;
        }
        if c == '"' {
            in_quotes = true;
        } else if c == delim {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    fields.push(cur);
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_empty_fields() {
        assert_eq!(split_line("a,b,c", ','), vec!["a", "b", "c"]);
        assert_eq!(split_line("a,,c", ','), vec!["a", "", "c"]);
        assert_eq!(split_line("", ','), vec![""]);
        assert_eq!(split_line("one", ','), vec!["one"]);
    }

    #[test]
    fn quoted_sections() {
        assert_eq!(
            split_line("a,\"b,c,d\",e", ','),
            vec!["a", "b,c,d", "e"]
        );
        assert_eq!(split_line("\"say \"\"hi\"\"\",x", ','), vec!["say \"hi\"", "x"]);
        // Delimiter inside quotes only: a quote toggling at line end still ends.
        assert_eq!(split_line("\"a;b\";c", ';'), vec!["a;b", "c"]);
    }

    #[test]
    fn crlf_tail_trimmed() {
        assert_eq!(split_line("a,b\r", ','), vec!["a", "b"]);
    }

    #[test]
    fn tab_and_pipe_delimiters() {
        assert_eq!(split_line("x\ty", '\t'), vec!["x", "y"]);
        assert_eq!(split_line("x|y|z", '|'), vec!["x", "y", "z"]);
    }
}
