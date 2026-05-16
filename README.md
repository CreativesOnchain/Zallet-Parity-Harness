# Zallet RPC Parity Harness

[![Zallet RPC Diff CI](https://github.com/CreativesOnchain/Zallet-Parity-Harness/actions/workflows/zallet-rpc-diff.yml/badge.svg)](https://github.com/CreativesOnchain/Zallet-Parity-Harness/actions/workflows/zallet-rpc-diff.yml)

A standalone parity-testing harness designed to compare the JSON-RPC outputs of `zcashd` and `Zallet`. 

As Zcash transitions from `zcashd` to Zallet, this tool ensures that downstream automation, exchanges, and services can confidently migrate by making wallet RPC differences measurable, reproducible, and actionable.

## Overview

The Zallet RPC Parity Harness (`zallet-rpc-diff`) runs a versioned suite of wallet RPC calls against two live endpoints (upstream `zcashd` and target `Zallet`). It normalizes the JSON responses, calculates exact diffs using RFC 6901 JSON Pointers, and classifies each method into one of five categories:

- ✅ **MATCH**: The normalized responses are completely identical.
- ❌ **DIFF**: The responses differ at specific, unexpected JSON paths.
- 📋 **EXPECTED_DIFF**: The responses differ, but the divergence is known and documented in the expected-differences configuration.
- 🔍 **MISSING**: One of the endpoints does not implement the method (typically Zallet).
- ⚠️ **ERROR**: A transport failure or an unexpected RPC error occurred.

## Quick Start

You will need Rust 1.75+ installed.

```bash
# 1. Clone the repository
git clone https://github.com/CreativesOnchain/Zallet-Parity-Harness.git
cd Zallet-Parity-Harness

# 2. Build the project
cargo build --release

# 3. Run the parity check
# Provide the RPC URLs for your zcashd node and your Zallet node
cargo run --release -- run \
  --upstream-url http://user:pass@127.0.0.1:8232 \
  --target-url http://user:pass@127.0.0.1:9067
```

*Note: For wallet methods to be comparable, both nodes must be on the same network and loaded with the exact same wallet keys.*

## Documentation

The project includes comprehensive documentation to help you use, configure, and understand the harness:

1. **[RUNBOOK.md](./RUNBOOK.md)** - The primary user manual. Contains detailed instructions on:
   - Prerequisites and installation
   - CLI flags and environment variable configuration
   - How to interpret the JSON and Markdown reports
   - How to extend the method suite (`manifest.toml`)
   - How to manage and suppress intentional differences (`expected_diffs.toml`)
   - Troubleshooting common issues

2. **[DESIGN_NOTE.md](./DESIGN_NOTE.md)** - The architectural overview. Explains the problem the tool solves, why normalization is necessary, and the logic behind the classification engine.

3. **[examples/](./examples/)** - Contains starter templates:
   - `endpoints.env`: Example environment variables for easy execution.
   - `expected_diffs.toml`: Example configuration for suppressing known, intentional JSON-RPC deviations.

## Output Example

When the tool finishes running, it outputs a detailed machine-readable `report.json` and a human-readable `report.md`. A typical summary looks like this:

```text
✅ Parity check complete!
   24 total | ✅ 18 match | ❌ 2 diff | 📋 3 expected | 🔍 1 missing | ⚠️ 0 error
   Report: report.json
   Report: report.md

⚠️  2 unexpected diff(s) found. Review 'report.json' for affected paths.
   If a diff is intentional, add it to your expected_diffs.toml.
```

## Contributing

Contributions to the method suite (`manifest.toml`) or the harness engine are welcome! Please ensure that any changes pass the CI suite:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
```
