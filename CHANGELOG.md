# Changelog

## 0.1.2 - 2026-07-29

- Prevented a second daemon from deleting the Unix socket of a running daemon.
- Preserved connectable sockets and regular files and refused startup.
- Removed only confirmed stale sockets.

## 0.1.1 - 2026-07-28

- Added configurable `idle_timeout_us` for high-speed GPIO polling. The default remains the 0.1.0-compatible 300 microseconds.
- Added `glitch_filter_us` so callers can select a filter appropriate for the signal's minimum pulse width.
- Rejected bus/operation mismatches before acquiring a lock.
- Added safety limits for transfers, edge counts, and wait times.
- Updated the Python client to 0.1.1.

## 0.1.0

- Added basic GPIO, I2C, SPI, and UART operations and GPIO edge monitoring.
- Added per-Unix-socket-client resource ownership and automatic cleanup on disconnect.
- Added Python-client reconnection without replaying an in-flight request.
