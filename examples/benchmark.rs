use std::env;
use std::hint::black_box;
use std::time::Instant;

use mstyle::formatter::{FormatterOptions, format_source};
use mstyle::source::SourceFile;

fn benchmark_source() -> String {
    let mut source = String::from("function y = benchmark_kernel(x)\n  y = x;\n");
    for index in 0..500 {
        source.push_str(&format!(
            "  y = y + x * {} - x / {};\n",
            index + 1,
            index + 2
        ));
    }
    source.push_str("end\n");
    source
}

fn main() {
    let iterations = env::var("MSTYLE_BENCH_ITERS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);
    let source_text = benchmark_source();
    let source = SourceFile::new("benchmark.m", source_text);
    let options = FormatterOptions::default();

    let warmup = format_source(&source, options).expect("benchmark warmup must format");
    black_box(warmup.output());

    let start = Instant::now();
    for _ in 0..iterations {
        let outcome = format_source(black_box(&source), options).expect("benchmark format failed");
        black_box(outcome.output());
    }
    let elapsed = start.elapsed();
    let bytes = source.len() * iterations;
    let mib_per_second = bytes as f64 / (1024.0 * 1024.0) / elapsed.as_secs_f64();

    println!(
        "mstyle formatter benchmark: {iterations} iterations, {} bytes/input, {:.2?}, {:.2} MiB/s",
        source.len(),
        elapsed,
        mib_per_second
    );
}
