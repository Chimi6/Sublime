# Benchmarks

One document per conversion pair, written like a methods section: what was
measured, how, under what constraints, and what the numbers do and do not
show. The harness that produces the numbers is in `bench/`; it is run by hand,
never in CI, and only on cause: a change to a converter's hot path, or a
release.

A pair is unordered (`csv-json` covers both directions) because the two
directions share a generator and each direction's output is the other's
input. Each direction is still its own converter with its own pass line,
reference pipeline, and results.

## Template

Every pair document has these sections, in this order.

1. **Purpose.** What the pair is and why its performance matters.
2. **Pass lines.** One row per direction plus the binary-wide targets. A pass
   line is a comparison against a named reference on the same machine in the
   same session, never an absolute number copied from elsewhere.
3. **Method.** Machine. How inputs are generated and their shape. The exact
   commands. How many runs and which statistic is reported. How memory,
   size, and startup are measured. What the reference pipelines are, with
   the file that implements them.
4. **Threats to validity.** Everything that could make the numbers mislead:
   synthetic data shape, page cache state, CPU power state, a single
   machine, choices made in the reference implementation.
5. **Results.** Dated blocks, newest first, each with the commit hash and
   machine line printed by the harness. Never edit an old block; add a new
   one.
6. **Conclusions.** What the numbers say to do next, and what they do not
   justify claiming.

## Recording a result

Run `bench/run.sh <pair>`, copy the printed table, commit line, and machine
line into a new results block, and note anything unusual about the run. If a
pass line fails, record it anyway; a failed row in the document is the
blocker entry in `STATE.md` pointing back here.
