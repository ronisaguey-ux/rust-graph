//! Compiles the representative grammars and prints what their tables cost.
//!
//! Run with `cargo run --release -p deep-tables --bin size-report`.

use deep_tables::grammars;

fn main() {
    let grammars = [
        ("expr", grammars::expr()),
        ("json", grammars::json()),
        ("python-subset", grammars::python_subset()),
    ];

    let mut total_raw = 0usize;
    let mut total_packed = 0usize;
    let mut total_states = 0usize;
    for (label, builder) in grammars {
        match builder.build() {
            Ok(compiled) => {
                let report = compiled.report();
                print!("{report}");
                println!(
                    "  bytes      {:>8} raw, {:>8} packed",
                    report.raw_bytes, report.packed_bytes
                );
                total_raw += report.raw_bytes;
                total_packed += report.packed_bytes;
                total_states += report.states;
            }
            Err(e) => println!("{label}: build failed: {e}"),
        }
    }

    println!();
    println!("totals: {total_states} states");
    println!("  raw    {total_raw:>8} bytes");
    println!("  packed {total_packed:>8} bytes");
    if total_packed > 0 {
        println!("  ratio  {:.2}x", total_raw as f64 / total_packed as f64);
    }
    println!();
    println!("10 MB budget: {:.1}% used by these three grammars", total_packed as f64 / (10.0 * 1024.0 * 1024.0) * 100.0);
}
