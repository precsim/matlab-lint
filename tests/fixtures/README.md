# Test fixtures

`formatter/` contains golden `input.m` / `expected.m` pairs for exact formatter output and idempotence.

`parser/` contains malformed and parser-ambiguity fixtures used to verify conservative recovery behavior.

The larger representative MATLAB corpus used for hardening lives in `tests/corpus/`. Corpus files are parser/formatter regression inputs; only syntax known to be compatible with the installed GNU Octave version is executed by the Octave suite.
