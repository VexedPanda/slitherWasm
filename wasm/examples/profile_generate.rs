//! Native profiling of puzzle generation on a dumped board (see profile_hard.rs for the dump format):
//! cargo run --release --features profile --example profile_generate <dump> <check: 0 none, 1 simple, 2 hard> [count]
use slither_core::cansolve::RuleSet;
use slither_core::generator::{Check, Generator, Rules, Status};
use slither_core::solver::Board;
use slither_core::topology::Topology;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let bytes = std::fs::read(&args[1]).unwrap();
    let check = match args[2].as_str() {
        "1" => Check::Simple,
        "2" => Check::Hard,
        _ => Check::None,
    };
    let count: u64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(3);
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
    let (nv, ne, nf, nh, num_sectors, clause_count) = (
        header[0] as usize,
        header[1] as usize,
        header[2] as usize,
        header[3] as usize,
        header[4] as usize,
        header[5] as usize,
    );
    let he = next();
    let vo = next();
    let vin = next();
    let fo = next();
    let fhe = next();
    let clauses = next();
    let topology = Topology::new(nv, ne, nf, &he, &vo, &vin, &fo, &fhe);
    // The SAT board, from the topology
    let mut edge_vertices = vec![0u32; 2 * ne];
    for h in 0..nh {
        let e = he[6 * h + 2] as usize;
        edge_vertices[2 * e] = he[6 * h];
        edge_vertices[2 * e + 1] = he[6 * h + 1];
    }
    let face_edges: Vec<u32> = fhe.iter().map(|&h| he[6 * h as usize + 2]).collect();
    let board = Board::new(nv, &edge_vertices, &fo, &face_edges);
    let rules = RuleSet::new(ne, num_sectors, &clauses, clause_count);
    // Optional easier (MEDIUM) rules, dumped after the face value sets
    let _faces = next();
    let easier_header = next();
    let easier_clauses = next();
    let easier = if easier_header.is_empty() {
        None
    } else {
        Some(RuleSet::new(
            ne,
            easier_header[0] as usize,
            &easier_clauses,
            easier_header[1] as usize,
        ))
    };
    let rule_sets = Rules {
        main: Some(&rules),
        easier: easier.as_ref(),
    };

    let start = std::time::Instant::now();
    for seed in 1..=count {
        let mut generator = Generator::new(&board, Some(&topology), rule_sets, check, seed * 7919, &[]);
        loop {
            generator.events.clear();
            match generator.step(&board, Some(&topology), rule_sets) {
                Status::Running => {}
                Status::Done => break,
                Status::Failed => panic!("failed"),
            }
        }
    }
    println!("{:.2} s per puzzle", start.elapsed().as_secs_f64() / count as f64);
    #[cfg(feature = "profile")]
    {
        let totals = unsafe { slither_core::generator::profile::TOTALS };
        for (name, total) in slither_core::generator::profile::NAMES.iter().zip(totals) {
            println!("  {:30} {:.2} s per puzzle", name, total / count as f64);
        }
        println!(
            "  counts per puzzle: {:?}",
            unsafe { slither_core::generator::profile::COUNTS }.map(|c| c / count)
        );
        let engine = unsafe { slither_core::engine::profile::TOTALS };
        for (name, total) in slither_core::engine::profile::NAMES.iter().zip(engine) {
            if total > 0.0 {
                println!("    HARD engine {:35} {:.2} s per puzzle", name, total / count as f64);
            }
        }
        println!("    HARD engine counts per puzzle: {:?}", unsafe { slither_core::engine::profile::COUNTS }.map(|c| c / count));
    }
}
