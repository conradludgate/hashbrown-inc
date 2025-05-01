use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use hashbrown::HashMap;
use hashbrown_inc::IncHashMap;
use hdrhistogram::Histogram;

fn main() {
    let hasher = std::hash::RandomState::new();

    println!("hashmap[incremental, siphash]:");
    let mut map = IncHashMap::with_hasher(hasher.clone());
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });

    println!("hashmap[non-incremental, siphash]:");
    let mut map = HashMap::with_hasher(hasher);
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });

    let hasher = foldhash::quality::RandomState::default();

    println!("hashmap[incremental, foldhash::quality]:");
    let mut map = IncHashMap::with_hasher(hasher.clone());
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });

    println!("hashmap[non-incremental, foldhash::quality]:");
    let mut map = HashMap::with_hasher(hasher);
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });

    let hasher = foldhash::fast::RandomState::default();

    println!("hashmap[incremental, foldhash::fast]:");
    let mut map = IncHashMap::with_hasher(hasher.clone());
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });

    println!("hashmap[non-incremental, foldhash::fast]:");
    let mut map = HashMap::with_hasher(hasher);
    map.extend((0..1000000).map(|k| (k, k)));
    map.shrink_to_fit();
    bench(100000, 100, map, |s| {
        s.get(&black_box(0));
    });
}

#[inline(never)]
fn bench<S>(rounds: usize, iterations: u32, mut state: S, mut f: impl FnMut(&mut S)) {
    let start = Instant::now();

    let h = {
        bench_inner(rounds, &mut || {
            let start = Instant::now();

            for _ in 0..iterations {
                f(black_box(&mut state))
            }

            start.elapsed() / iterations
        })
    };
    let dur = start.elapsed();

    for q in [0.5, 0.75, 0.9, 0.99, 0.999, 1.0] {
        println!(
            "\t{}'th percentile: {:?}",
            q * 100.0,
            Duration::from_nanos(h.value_at_quantile(q)),
        );
    }
    println!("\ttook {:?}", dur);
    println!()
}

#[inline(never)]
fn bench_inner(rounds: usize, f: &mut (dyn FnMut() -> Duration)) -> Histogram<u64> {
    let mut h = Histogram::<u64>::new(3).unwrap();

    for _ in 0..rounds {
        h += f().as_nanos() as u64;
    }

    h
}
