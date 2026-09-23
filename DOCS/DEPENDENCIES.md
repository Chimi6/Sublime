# Runtime Dependencies

Sublime ships with zero runtime dependencies. This file is the ledger. CI
fails if `Cargo.lock` contains a non-dev dependency that is not listed here.

Adding one requires:

1. A written justification: what correctness trap or capability it provides
   that we should not reimplement.
2. Measured cost on Linux x86_64: release binary size delta in bytes and
   clean-build time delta in seconds.
3. An entry in the table below.

| Crate | Version | Why | Binary size delta | Clean build delta | Added |
|---|---|---|---|---|---|
| (none) | | | | | |
