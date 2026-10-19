//! Native profiling of the HARD engine on dumped checks: cargo run --release --example profile_hard <dump>
use slither_core::cansolve::RuleSet;
use slither_core::engine::can_solve_hard;
use slither_core::topology::Topology;

fn main() {
    let path = std::env::args().nth(1).expect("dump path");
    let bytes = std::fs::read(path).unwrap();
    let words: Vec<u32> = bytes
        .chunks(4)
        .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    let mut i = 0;
    let mut next = || {
        let len = words[i] as usize;
        let slice = words[i + 1..i + 1 + len].to_vec();
        i += 1 + len;
        slice
    };
    let header = next();
    let (nv, ne, nf, _nh, num_sectors, clause_count, checks) = (
        header[0] as usize,
        header[1] as usize,
        header[2] as usize,
        header[3],
        header[4] as usize,
        header[5] as usize,
        header[6] as usize,
    );
    let he = next();
    let vo = next();
    let vin = next();
    let fo = next();
    let fhe = next();
    let clauses = next();
    let faces = next();
    let topology = Topology::new(nv, ne, nf, &he, &vo, &vin, &fo, &fhe);
    let rules = RuleSet::new(ne, num_sectors, &clauses, clause_count);
    let repeats: usize = std::env::args().nth(2).map(|s| s.parse().unwrap()).unwrap_or(1);
    let start = std::time::Instant::now();
    let mut solved = 0;
    for _ in 0..repeats {
        for c in 0..checks {
            let values: Vec<u8> = faces[c * nf..(c + 1) * nf].iter().map(|&v| v as u8).collect();
            if let Ok(true) = can_solve_hard(&topology, &rules, &values) {
                solved += 1;
            }
        }
    }
    let elapsed = start.elapsed();
    println!(
        "{} checks x{}, {} solved, {:.2} ms each",
        checks,
        repeats,
        solved,
        elapsed.as_secs_f64() * 1000.0 / (checks * repeats) as f64
    );
    #[cfg(feature = "profile")]
    for (name, total) in slither_core::engine::profile::NAMES
        .iter()
        .zip(unsafe { slither_core::engine::profile::TOTALS })
    {
        println!(
            "  {:35} {:.2} ms per check",
            name,
            total * 1000.0 / (checks * repeats) as f64
        );
    }
    #[cfg(feature = "profile")]
    println!(
        "  counts per check: {:?}",
        unsafe { slither_core::engine::profile::COUNTS }.map(|c| c / (checks * repeats) as u64)
    );
}
