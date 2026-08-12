# Pi4gpio design decisions

## 2026-08-05: Target the pigpiod ecosystem before the direct C ABI

Status: accepted

Pi4gpio will aim to become a functional superset of pigpio on Raspberry Pi 4,
but compatibility will be delivered in layers.

The first compatibility target is the public `pigpiod` ecosystem:

- the pigpio socket command protocol;
- the Python `pigpio.pi` API;
- the `pigs` command-line workflows;
- notifications, callbacks, PWM, servo pulses, and waves.

The in-process `libpigpio.so` C ABI is a later target.  Claiming complete pigpio
compatibility before that ABI and the documented edge cases pass conformance
tests would be misleading.

Pi4gpio's native protocol remains separate.  It may expose stronger guarantees
than the compatibility endpoint, including authenticated authorization,
per-session ownership, bounded operations, request identifiers, explicit safe
states, and no automatic replay of uncertain hardware mutations.

## 2026-08-05: One execution core, two protocol front ends

Status: accepted

The native Pi4gpio endpoint and the pigpio-compatible endpoint must translate
requests into the same internal command model and resource arbiter.  Hardware
must not be controlled independently by two protocol implementations.

The pigpio-compatible TCP listener is disabled by default.  When enabled it
must bind only to an explicitly configured address.  The recommended remote
deployment binds to a Tailscale address and applies Pi4gpio authorization on
top.  The native Unix socket remains available for local services.

## 2026-08-05: Separate hardware PWM from DMA-paced waves

Status: accepted

Pi4gpio will expose two timing backends on BCM2711:

1. The hardware PWM peripheral for GPIO 12, 13, 18, and 19 where the required
   alternate function is available.
2. A DMA-paced timeline engine for software PWM, servo pulses, arbitrary wave
   output, and accurately timestamped GPIO sampling on the remaining user GPIO.

Both backends reserve their GPIO and shared clock/DMA resources through the
same ownership system as GPIO, I2C, SPI, and UART.  Low-level register, DMA, and
memory-mapping code remains isolated in the hardware crate.

## 2026-08-05: Safety is part of the superset claim

Status: accepted

Pi4gpio will not define "superset" as a larger function list alone.  Native
clients must additionally receive:

- per-client and per-resource authorization;
- bounded waveform uploads and execution time;
- atomic wave compilation before GPIO state changes;
- explicit cancellation and a configured safe state on disconnect or watchdog
  expiry;
- content hashes for uploaded wave programs;
- idempotency keys and short-lived result caching for remote mutations;
- observable ownership, contention, underrun, overrun, and timing-error metrics.

Exactly-once execution cannot be guaranteed across every network and process
failure.  APIs must report an indeterminate outcome rather than silently
replaying a mutation whose completion is unknown.
