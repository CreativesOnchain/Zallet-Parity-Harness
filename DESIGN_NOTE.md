# Design Note: `zallet-rpc-diff`

## 1. Result Categories

Each RPC method tested will be classified into one of the following categories:

| Category | Description |
| :--- | :--- |
| `MATCH` | Both endpoints returned identical (normalized) JSON results. |
| `DIFF` | Both endpoints returned results, but they differ after normalization. |
| `EXPECTED_DIFF` | A difference was found, but it matches an entry in `expected_diffs.toml` (intentional divergence). |
| `MISSING` | The method returned `-32601` (method not found) on one or both endpoints. |
| `ERROR` | A transport error or internal failure occurred during execution. |

## 2. Report Schema (`report.json`)

The output report follows this JSON structure:

```json
{
  "schema_version": "1",
  "generated_at": "2024-01-15T12:34:56Z",
  "summary": {
    "total": 25,
    "matches": 18,
    "diffs": 2,
    "expected_diffs": 3,
    "missing": 1,
    "errors": 1
  },
  "details": {
    "getblockchaininfo": {
      "type": "match"
    },
    "z_gettotalbalance": {
      "type": "diff",
      "diff_count": 1,
      "diff_paths": ["/private"]
    },
    "getnetworkinfo": {
      "type": "expected_diff",
      "diff_count": 2,
      "diff_paths": ["/version", "/subversion"],
      "reason": "Zallet reports its own version string."
    },
    "z_getaddressforaccount": {
      "type": "missing",
      "method": "z_getaddressforaccount"
    }
  }
}
```

**Notes:**
- `schema_version` is `"1"` and will increment on breaking schema changes.
- `generated_at` is an ISO-8601 UTC timestamp (seconds precision).
- `details` is ordered lexicographically by method name for deterministic, diff-friendly output.
- `diff_paths` uses JSON Pointer notation (RFC 6901): `/field`, `/array/0/key`.

## 3. Method-Suite Manifest (`manifest.toml`)

The manifest defines which RPC calls to run and lives at `manifest.toml` (or `--manifest <path>`).

```toml
[[methods]]
name = "getbalance"
params = []
tags = ["wallet", "balance"]
ignore_paths = []
```

**Field reference:**

| Field | Type | Required | Description |
| :--- | :--- | :--- | :--- |
| `name` | string | ✅ | JSON-RPC method name |
| `params` | JSON value | ❌ | Parameters to pass. `null`/absent → no params. Array → positional args. Scalar/object → single arg. |
| `ignore_paths` | string[] | ❌ | RFC 6901 paths to strip from both responses before comparison |
| `tags` | string[] | ❌ | Free-form labels for filtering (not used by engine) |

## 4. Params Encoding

The client encodes manifest `params` to JSON-RPC wire format as follows:

| Manifest `params` value | Wire format |
| :--- | :--- |
| `null` or absent | Empty params (`[]`) |
| `["addr", 1]` (array) | Each element becomes a positional argument: `["addr", 1]` |
| `"myaddr"` (scalar or object) | Wrapped in a single arg: `["myaddr"]` |

This matches the calling convention expected by real `zcashd`/`Zallet` endpoints.

## 5. Bounded Concurrency

The engine uses a Tokio `Semaphore` to cap simultaneous in-flight RPC pairs.
The limit defaults to **8** and is configurable via `--concurrency N`.
Setting `--concurrency 1` produces a serial run, which is useful for debugging noisy nodes.

## 6. MISSING vs EXPECTED_DIFF

Currently, methods that return `-32601` (method not found) are always classified as
`MISSING`, even if listed in `expected_diffs.toml`. The `expected_diffs.toml` file is
intended for methods where **both** endpoints respond but their results differ.

If you want to track methods that are expected to be missing on Zallet, document them
in a separate section of `expected_diffs.toml` with a comment, and note that the engine
will still report them as `MISSING` (which is accurate). A future iteration may add
first-class support for `EXPECTED_MISSING`.
