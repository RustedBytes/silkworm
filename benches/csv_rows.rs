// Compare the production row encoder with its implementation at 5fb1d5b.
#[allow(dead_code, unused_imports)]
#[path = "../src/pipelines/csv.rs"]
mod csv;
use std::hint::black_box;
use std::time::Instant;

fn previous_row<'a>(buffer: &mut Vec<u8>, fields: impl Iterator<Item = &'a str>) {
    for (index, field) in fields.enumerate() {
        if index > 0 {
            buffer.push(b',');
        }
        let needs_quotes = field.contains(',')
            || field.contains('"')
            || field.contains('\n')
            || field.contains('\r');
        let encoded = if needs_quotes {
            format!("\"{}\"", field.replace('"', "\"\""))
        } else {
            field.to_string()
        };
        buffer.extend_from_slice(encoded.as_bytes());
    }
    buffer.push(b'\n');
}

pub fn scenarios(mut measure: impl FnMut(&str, &mut dyn FnMut())) {
    for (case, field) in [
        ("plain", "plain value".to_string()),
        ("quoted", "їжак, \"rust\"\r\n".to_string()),
        ("large", "довгий текст, \"rust\" ".repeat(64)),
    ] {
        for count in [1, 8, 64] {
            let fields = vec![field.as_str(); count];
            let mut previous = Vec::new();
            previous_row(&mut previous, fields.iter().copied());
            let mut output = Vec::with_capacity(previous.len());
            csv::append_row(&mut output, fields.iter().copied());
            assert_eq!(output, previous);
            for baseline in [true, false] {
                let name = format!(
                    "{case}_{count}_{}",
                    if baseline { "before" } else { "after" }
                );
                measure(&name, &mut || {
                    for _ in 0..20_000 {
                        output.clear();
                        if baseline {
                            previous_row(&mut output, black_box(fields.iter().copied()));
                        } else {
                            csv::append_row(&mut output, black_box(fields.iter().copied()));
                        }
                        black_box(&output);
                    }
                });
            }
        }
    }
}

fn main() {
    // Alternate before/after measurements within every sampling round.
    for sample in 0..9 {
        scenarios(|name, work| {
            if sample == 0 {
                for _ in 0..2 {
                    work();
                }
            }
            let start = Instant::now();
            work();
            println!(
                "case={name} sample={sample} operations=20000 ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.
            );
        });
    }
}
