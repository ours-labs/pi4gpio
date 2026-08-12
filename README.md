# Pi4gpio

Pi4gpio is a local hardware-access daemon for Raspberry Pi 4. It provides a shared API for GPIO, I2C, SPI, and UART while enforcing per-client resource ownership.

## Released capabilities

Version 0.1.2 provides:

- GPIO input, output, and timestamped edge monitoring
- I2C, SPI, and UART byte operations
- per-connection ownership and automatic cleanup after disconnect
- bounded transfer sizes, wait durations, and edge counts
- a Unix-domain socket transport
- a dependency-free Python client

Hardware PWM, servo pulses, waveform generation, remote control, and pigpiod protocol compatibility are not available in 0.1.2. The precise support boundary is documented in [docs/CAPABILITIES.md](docs/CAPABILITIES.md).

## Unreleased functionality

The current `main` branch adds a read-only `hello` operation for native protocol version and capability discovery. Existing version 1 hardware requests keep their original shape. This operation is not part of the 0.1.2 release and does not provide pigpiod wire-protocol compatibility.

## Security boundary

The protocol is local-only. Do not expose the Unix socket through a TCP proxy or VPN bridge. A future remote transport requires an independently reviewed authentication, authorization, replay-protection, rate-limit, and audit design.

Pi4gpio arbitrates participating clients, but it cannot prevent another process from opening hardware devices directly. Use a dedicated service account and operating-system device permissions to keep direct access outside the daemon.

See [docs/SECURITY_MODEL.md](docs/SECURITY_MODEL.md) for the public threat model and deployment requirements.

## Responsibilities

The daemon owns raw bus operations, resource arbitration, and disconnect cleanup. Sensor decoding, value validation, persistence, and sampling schedules remain client responsibilities.

## Repository layout

- `crates/pi4gpio-daemon`: Unix-socket server and resource arbiter
- `crates/pi4gpio-hw`: Linux hardware access
- `clients/python`: `pi4gpio_client` package
- `systemd`: generic service example
- `docs`: public capability and security documentation

## Build

For Raspberry Pi 4 using a 64-bit operating system:

```bash
cargo build --release --target aarch64-unknown-linux-gnu
```

## Test

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
PYTHONPATH=clients/python python3 -m unittest discover -s clients/python/tests -v
python3 -m unittest discover -s protocol/tests -v
```

Hardware acceptance results should identify the Pi model, operating-system version, architecture, tested interfaces, duration, sample count, error count, and software versions without publishing hostnames, addresses, account names, or local paths.

## License

[MIT](LICENSE)
