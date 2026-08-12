# Pi4gpio

Pi4gpio is a Raspberry Pi 4 hardware-access daemon written in Rust. It provides
a shared local API for GPIO, I2C, SPI, and UART while arbitrating resources
between clients.

## Current status

The current daemon supports:

- GPIO input/output and timestamped edge monitoring
- I2C, SPI, and UART byte operations
- per-connection resource ownership and automatic cleanup on disconnect
- bounded transfer, wait, and edge-count requests
- a local Unix-domain socket API
- a dependency-free Python client

The following features are roadmap work and are not part of release 0.1.2:

- TCP or other network listeners, remote authentication, and API keys
- hardware PWM, servo pulses, and waveform generation
- pigpiod protocol compatibility
- bit-banged I2C/SPI, 1-Wire, and a script engine

Do not expose the current Unix-socket protocol through a network bridge. See
[NETWORK_POLICY.md](NETWORK_POLICY.md) for the proposed security boundary and
[PIGPIO_SUPERSET_ROADMAP.md](PIGPIO_SUPERSET_ROADMAP.md) for planned work.

## Responsibilities

The daemon owns raw hardware operations, per-connection arbitration, and
cleanup. Sensor decoding, validation, persistence, and sampling schedules
belong to client applications.

Pi4gpio cannot detect a process that bypasses the daemon and opens a hardware
device directly. Deployments that require exclusive access should combine
Pi4gpio with operating-system restrictions described in
[EXCLUSIVE_ACCESS.md](EXCLUSIVE_ACCESS.md).

## Repository layout

- `crates/pi4gpio-daemon` — `pi4gpiod`, the Unix-socket server and resource arbiter
- `crates/pi4gpio-hw` — Linux hardware access with platform-specific unsafe code kept local
- `clients/python` — the `pi4gpio_client` Python package
- `systemd/pi4gpio.service` — a generic service example

## Build

For Raspberry Pi 4 (`aarch64-unknown-linux-gnu`):

```bash
cargo build --release --target aarch64-unknown-linux-gnu
```

## Test

Run the hardware-independent suite on Linux:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
PYTHONPATH=clients/python python3 -m unittest discover -s clients/python/tests -v
```

Hardware claims require a separately documented, reproducible fixture. Private
deployment logs and environment-specific acceptance records are not stored in
this public repository.

## Documentation

- [docs/DECISIONS.md](docs/DECISIONS.md) — architectural decisions
- [PIGPIO_SUPERSET_ROADMAP.md](PIGPIO_SUPERSET_ROADMAP.md) — compatibility roadmap
- [FEATURE_PRIORITY.md](FEATURE_PRIORITY.md) — implemented foundations and future scope
- [EXCLUSIVE_ACCESS.md](EXCLUSIVE_ACCESS.md) — operating-system enforcement guidance
- [NETWORK_POLICY.md](NETWORK_POLICY.md) — network exposure policy

## License

[MIT](LICENSE)
