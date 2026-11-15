#!/usr/bin/env python3

import os
import pathlib
import statistics
import subprocess
import tempfile
import time


ITERATIONS = 5
TARGET_SECONDS = 0.5
REQUIRED_SECONDS = 1.0


def run(cwd: pathlib.Path, *args: str, env=None) -> None:
    completed = subprocess.run(
        args,
        cwd=cwd,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            f"{' '.join(args)} failed with {completed.returncode}\n"
            f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
        )


def source(version: int) -> str:
    lines = [
        "function y = benchmark_diff(x)",
        "  y = x;",
    ]
    for index in range(120):
        adjustment = version if index % 20 == 0 else 0
        lines.append(f"  y = y + x * {index + 1 + adjustment};")
    lines.append("end")
    return "\n".join(lines) + "\n"


def write_corpus(root: pathlib.Path, version: int) -> None:
    for index in range(5):
        (root / f"kernel_{index}.m").write_text(
            source(version + index),
            encoding="utf-8",
        )


def setup_git(parent: pathlib.Path) -> pathlib.Path:
    root = parent / "git"
    root.mkdir()
    run(root, "git", "init", "-q")
    run(root, "git", "config", "user.email", "test@example.invalid")
    run(root, "git", "config", "user.name", "mstyle benchmark")
    write_corpus(root, 0)
    run(root, "git", "add", ".")
    run(root, "git", "commit", "-qm", "base")
    write_corpus(root, 1)
    return root


def setup_fossil(parent: pathlib.Path) -> tuple[pathlib.Path, dict[str, str]]:
    repository = parent / "benchmark.fossil"
    root = parent / "fossil"
    root.mkdir()
    env = os.environ.copy()
    env["FOSSIL_USER"] = "mstyle-benchmark"
    run(parent, "fossil", "init", str(repository), env=env)
    run(root, "fossil", "open", str(repository), env=env)
    run(root, "fossil", "settings", "autosync", "off", env=env)
    write_corpus(root, 0)
    run(root, "fossil", "add", ".", env=env)
    run(
        root,
        "fossil",
        "commit",
        "--nosync",
        "--no-prompt",
        "-m",
        "base",
        env=env,
    )
    write_corpus(root, 1)
    return root, env


def benchmark(binary: pathlib.Path, root: pathlib.Path, vcs: str, env=None) -> list[float]:
    command = [str(binary), "diff-check", "--vcs", vcs, "--format", "json"]
    run(root, *command, env=env)
    samples = []
    for _ in range(ITERATIONS):
        start = time.perf_counter()
        run(root, *command, env=env)
        samples.append(time.perf_counter() - start)
    return samples


def report(name: str, samples: list[float]) -> None:
    median = statistics.median(samples)
    worst = max(samples)
    target = "meets target" if median < TARGET_SECONDS else "above target"
    print(
        f"{name}: median={median * 1000:.1f} ms, "
        f"worst={worst * 1000:.1f} ms ({target}; target <500 ms)"
    )
    if worst >= REQUIRED_SECONDS:
        raise RuntimeError(
            f"{name} diff-check exceeded required <1 s ceiling: {worst:.3f} s"
        )


def main() -> None:
    repository_root = pathlib.Path(__file__).resolve().parents[1]
    binary = repository_root / "target" / "release" / "mstyle"
    if not binary.is_file():
        raise RuntimeError(f"release binary not found: {binary}")

    with tempfile.TemporaryDirectory(prefix="mstyle-diff-benchmark-") as temp:
        parent = pathlib.Path(temp)
        git_root = setup_git(parent)
        fossil_root, fossil_env = setup_fossil(parent)
        report("git", benchmark(binary, git_root, "git"))
        report("fossil", benchmark(binary, fossil_root, "fossil", fossil_env))


if __name__ == "__main__":
    main()
