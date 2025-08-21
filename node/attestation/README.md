# TDX Attestation

This crate provides TDX attestation with runtime selection between hardware and simulation.

## Test Environment Control

Tests use the `TDX_TESTS_REQUIRE_HARDWARE` environment variable to determine TDX interface selection:

### Production/CI Testing: Hardware Required

Forces TDX hardware usage and prevents simulation fallback.

```bash
TDX_TESTS_REQUIRE_HARDWARE=true cargo test
# or
TDX_TESTS_REQUIRE_HARDWARE=1 cargo test
```

- **Panics immediately** if TDX hardware unavailable
- **Never uses simulation** when hardware is explicitly requested
- **Cannot be bypassed** at runtime - prevents test misconfiguration

### Development Mode (default)

Uses simulation for testing on non-TDX systems.

```bash
cargo test
# or explicitly:
TDX_TESTS_REQUIRE_HARDWARE=false cargo test
```

- Always uses simulation for development
- No hardware dependencies
- Enables development on non-TDX machines

## Usage

| Environment            | Test Command                                  | Behavior                            |
| ---------------------- | --------------------------------------------- | ----------------------------------- |
| TDX Production/CI      | `TDX_TESTS_REQUIRE_HARDWARE=true cargo test`  | Hardware only, panic if unavailable |
| Development            | `cargo test`                                  | Simulation only                     |
| Development (explicit) | `TDX_TESTS_REQUIRE_HARDWARE=false cargo test` | Simulation only                     |

**Important**: Simulation is NEVER used when `TDX_TESTS_REQUIRE_HARDWARE=true`. Tests will fail fast with clear error messages if TDX hardware is requested but unavailable.
