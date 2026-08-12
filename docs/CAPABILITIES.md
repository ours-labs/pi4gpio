# Capabilities

This document describes product behavior, not a delivery schedule. A capability is listed as available only after its implementation, security controls, automated tests, and user documentation are complete.

| Area | 0.1.2 status | Notes |
| --- | --- | --- |
| GPIO input and output | Available | BCM GPIO numbering |
| GPIO edge monitoring | Available | Timestamped, bounded edge collection |
| I2C transfers | Available | Kernel `i2c-dev` backend |
| SPI transfers | Available | Kernel `spidev` backend |
| UART transfers | Available | Kernel `termios` backend |
| Resource ownership | Available | Scoped to one client connection |
| Disconnect cleanup | Available | Releases resources owned by the disconnected session |
| Python client | Available | Local Unix-socket client |
| Hardware PWM and servo pulses | Not available | No public API or timing guarantee |
| Waveform generation | Not available | No queued pulse or DMA waveform engine |
| Remote control | Not available | No network listener or remote authentication |
| pigpiod protocol compatibility | Not available | Native Pi4gpio protocol only |
| Bit-banged buses and script engine | Not available | Not part of the current public contract |

## Unreleased functionality on `main`

The source on `main` includes a bus-free, read-only `hello` operation for native protocol version and capability discovery. It preserves existing version 1 hardware request formats and does not acquire GPIO, I2C, SPI, or UART resources. This functionality is not included in the 0.1.2 release artifacts and does not provide pigpiod wire-protocol compatibility.

Unsupported features must not be inferred from internal types, experimental branches, or issue discussions.
