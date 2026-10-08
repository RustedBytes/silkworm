/// Append a complete CSV row to reusable output storage.
pub(super) fn append_row<'a>(buffer: &mut Vec<u8>, fields: impl Iterator<Item = &'a str>) {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            buffer.push(b',');
        }
        append_field(buffer, field);
    }
    buffer.push(b'\n');
}

fn append_field(buffer: &mut Vec<u8>, field: &str) {
    let bytes = field.as_bytes();
    if !bytes
        .iter()
        .any(|byte| matches!(byte, b',' | b'"' | b'\n' | b'\r'))
    {
        buffer.extend_from_slice(bytes);
        return;
    }
    buffer.push(b'"');
    // Copy contiguous UTF-8 bytes unchanged, doubling only ASCII quotes.
    for (index, part) in field.split('"').enumerate() {
        if index > 0 {
            buffer.extend_from_slice(b"\"\"");
        }
        buffer.extend_from_slice(part.as_bytes());
    }
    buffer.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::append_row;

    #[test]
    fn rows_preserve_unicode_quotes_and_empty_fields() {
        let mut output = b"prefix\n".to_vec();
        append_row(&mut output, ["", "їжак", "a,b", "\"", "\r\n"].into_iter());
        assert_eq!(
            output,
            "prefix\n,їжак,\"a,b\",\"\"\"\",\"\r\n\"\n".as_bytes()
        );
        output.clear();
        append_row(&mut output, std::iter::empty());
        assert_eq!(output, b"\n");
    }

    #[test]
    fn escaping_matches_previous_encoder_for_generated_fields() {
        let alphabet = ["a", ",", "\"", "\r", "\n", "ї", "🦀"];
        for length in 0..=4 {
            for mut code in 0..alphabet.len().pow(length) {
                let mut field = String::new();
                for _ in 0..length {
                    field.push_str(alphabet[code % alphabet.len()]);
                    code /= alphabet.len();
                }
                let expected = if field.contains([',', '"', '\n', '\r']) {
                    format!("\"{}\"\n", field.replace('"', "\"\""))
                } else {
                    format!("{field}\n")
                };
                let mut output = Vec::new();
                append_row(&mut output, std::iter::once(field.as_str()));
                assert_eq!(output, expected.as_bytes(), "field={field:?}");
            }
        }
    }
}
