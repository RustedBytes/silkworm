// Measures production URL preparation; fixtures are outside the measured region.
// The benchmark and isolated allocation harness include this private module.
#[allow(dead_code, unused_imports)]
#[path = "../src/http/query.rs"]
mod query;
#[allow(unused_imports)]
pub use silkworm::{errors, types};
use std::hint::black_box;
use std::time::Instant;
use types::Params;

pub fn scenarios(mut measure: impl FnMut(&str, &mut dyn FnMut())) {
    for count in [0, 1, 8, 64] {
        let params: Params = (0..count)
            .map(|i| (format!("param-{i:03}"), "їжак / rust + & ? = ".repeat(4)))
            .collect();
        for canonical in [false, true] {
            let name = format!(
                "{}_{count}_params",
                if canonical { "canonical" } else { "request" }
            );
            measure(&name, &mut || {
                for _ in 0..20_000 {
                    let result = if canonical {
                        query::canonical_url_with_params(
                            black_box("https://example.com/path?tag=b&tag=a&param-000=old#top"),
                            black_box(&params),
                        )
                    } else {
                        query::build_url_with_params(
                            black_box("https://example.com/path?tag=b&tag=a&param-000=old#top"),
                            black_box(&params),
                        )
                    };
                    black_box(result.expect("valid benchmark URL"));
                }
            });
        }
    }
    let params = Params::from([(String::new(), String::new()), ("a".into(), "b".into())]);
    for (name, url) in [
        ("empty_fields", "https://example.com/?=old&a=1&a=2"),
        ("invalid_url", "not a URL"),
    ] {
        measure(name, &mut || {
            for _ in 0..20_000 {
                let _ = black_box(query::canonical_url_with_params(
                    black_box(url),
                    black_box(&params),
                ));
            }
        });
    }
}
fn main() {
    scenarios(|name, work| {
        for _ in 0..2 {
            work();
        }
        for sample in 0..9 {
            let start = Instant::now();
            work();
            println!(
                "case={name} sample={sample} operations=20000 ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.
            );
        }
    });
}
