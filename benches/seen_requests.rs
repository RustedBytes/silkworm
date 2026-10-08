// Production cache microbenchmark. Fixture setup is outside the measured boundary.
// The no-harness benchmark does not run the imported module's unit tests.
#[allow(dead_code, unused_imports)]
#[path = "../src/engine/seen.rs"]
mod seen;
use seen::SeenRequests;
use std::hint::black_box;
use std::time::Instant;

pub fn scenarios(mut measure: impl FnMut(&str, &mut dyn FnMut())) {
    for key_len in [32usize, 128, 1024] {
        let keys: Vec<_> = (0..20_000)
            .map(|i| {
                let mut key = format!("GET https://example.com/{i:05}/");
                key.extend(std::iter::repeat_n('x', key_len.saturating_sub(key.len())));
                key
            })
            .collect();
        for (label, limit) in [("bounded_cold", Some(keys.len())), ("unbounded_cold", None)] {
            measure(&format!("{label}_key{key_len}"), &mut || {
                let mut cache = SeenRequests::new(limit);
                for key in &keys {
                    black_box(cache.insert_if_new(black_box(key)));
                }
                // Cold cases include cache construction, growth and destruction.
                black_box(cache);
            });
        }
        let mut rotating = SeenRequests::new(Some(4096));
        for key in &keys[keys.len() - 4096..] {
            rotating.insert_if_new(key);
        }
        measure(&format!("bounded_eviction_key{key_len}"), &mut || {
            for key in &keys {
                black_box(rotating.insert_if_new(black_box(key)));
            }
        });
        let mut hits = SeenRequests::new(Some(4096));
        for key in &keys[..4096] {
            hits.insert_if_new(key);
        }
        measure(&format!("bounded_hits_key{key_len}"), &mut || {
            for i in 0..keys.len() {
                black_box(hits.insert_if_new(black_box(&keys[i % 4096])));
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
