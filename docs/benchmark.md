# Formatter benchmark

Run the end-to-end parser + formatter microbenchmark in release mode:

```bash
cargo run --locked --release --example benchmark
```

Set `MSTYLE_BENCH_ITERS` to control the iteration count:

```bash
MSTYLE_BENCH_ITERS=1000 cargo run --locked --release --example benchmark
```

The benchmark formats a generated MATLAB function with 500 arithmetic statements and reports input size, elapsed time, and approximate MiB/s. It is a regression/measurement tool, not a hard CI performance threshold.
