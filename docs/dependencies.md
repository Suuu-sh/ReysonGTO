# M1 dependency license audit

Audit date: 2026-10-15. Scope: every external package in the checked-in
`Cargo.lock`, including build/dev dependencies and platform-specific dependencies.

## Method and result

Executed `cargo metadata --format-version 1 --locked` without `--no-deps` or a
platform filter. Compared every registry package `(name, version)` with
`Cargo.lock`: **35 packages, no missing or extra entries**.

This is a manifest-license metadata audit equivalent to the requested minimum
license check, not a `cargo deny` run or a full source/legal review. Only package
metadata and `unicode-ident` license notices were inspected; no poker library
source was read. No poker evaluator or solver dependency is present.

Allowed license identifiers: MIT, Apache-2.0, Unlicense, Unicode-3.0.
All resolved expressions below use only those permissive identifiers. No GPL,
AGPL, LGPL, unknown, or absent license declaration was found. For MIT/Apache
alternatives either license can be selected. `memchr` can use MIT rather than
Unlicense. `unicode-ident` additionally requires the Unicode-3.0 notice:
its bundled `LICENSE-UNICODE` permits use/modification/distribution with retained
copyright/permission notice; this is not a copyleft requirement. Distribution
must retain the applicable license notices.

The default Cargo registry is read-only in this environment, so the full metadata
command used an isolated registry cache (no credentials/config copied):

```sh
CARGO_HOME=/tmp/reyson-m1-audit-cargo cargo metadata --format-version 1 --locked
```

Raw metadata evidence: `/tmp/reyson-m1-metadata.json` (local, not committed).
Re-run this audit after changing dependencies or the lockfile.

## Locked external packages

“Direct” includes workspace build/dev dependencies; “Transitive” includes
Windows-only packages even though the verification machine is macOS.

| Package | Version | Relationship | Manifest license expression |
| --- | --- | --- | --- |
| `anstream` | 1.0.0 | Transitive | `MIT OR Apache-2.0` |
| `anstyle` | 1.0.14 | Transitive | `MIT OR Apache-2.0` |
| `anstyle-parse` | 1.0.0 | Transitive | `MIT OR Apache-2.0` |
| `anstyle-query` | 1.1.5 | Transitive | `MIT OR Apache-2.0` |
| `anstyle-wincon` | 3.0.11 | Transitive | `MIT OR Apache-2.0` |
| `anyhow` | 1.0.104 | Direct | `MIT OR Apache-2.0` |
| `clap` | 4.6.7 | Direct | `MIT OR Apache-2.0` |
| `clap_builder` | 4.6.7 | Transitive | `MIT OR Apache-2.0` |
| `clap_derive` | 4.6.7 | Transitive | `MIT OR Apache-2.0` |
| `clap_lex` | 1.1.1 | Transitive | `MIT OR Apache-2.0` |
| `colorchoice` | 1.0.5 | Transitive | `MIT OR Apache-2.0` |
| `crossbeam-deque` | 0.8.8 | Transitive | `MIT OR Apache-2.0` |
| `crossbeam-epoch` | 0.9.21 | Transitive | `MIT OR Apache-2.0` |
| `crossbeam-utils` | 0.8.23 | Transitive | `MIT OR Apache-2.0` |
| `either` | 1.19.0 | Transitive | `MIT OR Apache-2.0` |
| `heck` | 0.5.0 | Transitive | `MIT OR Apache-2.0` |
| `is_terminal_polyfill` | 1.70.2 | Transitive | `MIT OR Apache-2.0` |
| `itoa` | 1.0.18 | Transitive | `MIT OR Apache-2.0` |
| `memchr` | 2.8.3 | Transitive | `Unlicense OR MIT` |
| `once_cell_polyfill` | 1.70.2 | Transitive | `MIT OR Apache-2.0` |
| `proc-macro2` | 1.0.107 | Transitive | `MIT OR Apache-2.0` |
| `quote` | 1.0.47 | Transitive | `MIT OR Apache-2.0` |
| `rayon` | 1.12.0 | Direct | `MIT OR Apache-2.0` |
| `rayon-core` | 1.13.0 | Transitive | `MIT OR Apache-2.0` |
| `serde` | 1.0.229 | Direct | `MIT OR Apache-2.0` |
| `serde_core` | 1.0.229 | Transitive | `MIT OR Apache-2.0` |
| `serde_derive` | 1.0.229 | Transitive | `MIT OR Apache-2.0` |
| `serde_json` | 1.0.154 | Direct | `MIT OR Apache-2.0` |
| `strsim` | 0.11.1 | Transitive | `MIT` |
| `syn` | 3.0.9 | Transitive | `MIT OR Apache-2.0` |
| `unicode-ident` | 1.0.27 | Transitive | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| `utf8parse` | 0.2.2 | Transitive | `Apache-2.0 OR MIT` |
| `windows-link` | 0.2.1 | Transitive | `MIT OR Apache-2.0` |
| `windows-sys` | 0.61.2 | Transitive | `MIT OR Apache-2.0` |
| `zmij` | 1.0.23 | Transitive | `MIT` |

## M2 changes

Tree merge configuration and benchmark-range export add no external dependencies.
`tools/export_benchmark_ranges.py` and its synthetic tests use only Python standard
library modules; the exporter reads saved JSON, never product/OSS source code.
The existing locked-package license audit remains unchanged.
