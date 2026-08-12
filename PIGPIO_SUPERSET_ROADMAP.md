# Pigpio superset roadmap for Raspberry Pi 4

Date: 2026-08-05

## Goal

Make Pi4gpio the preferred pigpio-compatible control service on Raspberry Pi 4
by combining pigpio migration compatibility with stronger isolation, remote
authorization, ownership, recovery, diagnostics, and safe-state behavior.

The initial compatibility claim is deliberately scoped to the `pigpiod`
client/server ecosystem.  Full source or binary compatibility with the direct
in-process C API is a later milestone.

## Architecture

```text
                    native Unix socket
Pi4gpio clients  --------------------------+
                                             v
                                      +-------------+
pigpio.py / pigs -- pigpio protocol ->| protocol    |
                                      | adapters    |
remote native clients -- secure TCP ->|             |
                                      +------+------+
                                             |
                                normalized commands/events
                                             |
                    +------------------------+-----------------------+
                    | resource ownership + authorization + quotas   |
                    +------------------------+-----------------------+
                                             |
             +-------------------------------+--------------------+
             |                               |                    |
      ordinary GPIO/I2C/SPI/UART      PWM peripheral       timeline engine
      kernel devices + gpiomem         GPIO 12/13/18/19     DMA + PCM/PWM pace
             |                               |                    |
             +-------------------------------+--------------------+
                                             |
                                           BCM2711
```

The internal command model is the only path to hardware.  Compatibility and
native endpoints never maintain separate GPIO state.

## Compatibility definition

Four labels prevent an incomplete implementation from being advertised as a
drop-in replacement:

- `native`: Pi4gpio's own API and safety contract.
- `pigpiod-compatible subset`: listed pigpio commands pass conformance tests.
- `pigpiod-compatible`: all documented daemon commands, notifications, and
  client-visible error behavior pass, except explicitly documented unsafe
  administrative operations.
- `pigpio-compatible`: the daemon interfaces plus the direct C source/ABI and
  timing behavior pass.  This is the final target.

The `shell`, unrestricted file access, and similar pigpio daemon operations are
not exposed remotely by default.  If strict compatibility eventually requires
them, they run only in an explicitly enabled and sandboxed compatibility mode.

## Milestones and gates

### M0 - Protocol and timing specification

Deliverables:

- versioned native protocol with capability negotiation;
- internal command, event, handle, ownership, and error types;
- machine-readable pigpio command support matrix;
- monotonic clock and timestamp conversion rules;
- conformance harness that can run official pigpio client tests against either
  `pigpiod` or `pi4gpiod` and compare results.

Gate: existing local Unix clients remain compatible and all current tests pass.

### M1 - Secure remote control

Deliverables:

- optional length-prefixed native TCP transport;
- explicit bind address with wildcard binds rejected by default;
- Tailscale-address deployment profile;
- hashed API credentials with scopes for pins, buses, reads, writes, PWM,
  waves, and administration;
- session IDs, request IDs, quotas, rate limits, audit records, and key
  rotation/revocation;
- event streaming on a separate logical channel so callbacks cannot block
  command responses.

Gate: unauthorized requests cannot acquire a resource; disconnect releases
resources and applies configured safe states; uncertain mutations are not
automatically replayed.

### M2 - Hardware PWM and servo foundation

Deliverables:

- BCM2711 PWM peripheral backend;
- clock divisor, frequency, range, duty cycle, polarity, and channel conflict
  validation;
- hardware PWM on supported alternate-function pins;
- software-facing PWM and servo APIs whose handles participate in ownership;
- restoration or safe-state policy for stop, disconnect, and daemon exit.

Gate: frequency and duty-cycle error are measured with a logic analyzer across
the documented range; conflicting clock or channel requests fail before GPIO
state changes.

### M3 - DMA-paced timeline engine

Deliverables:

- isolated BCM2711 DMA memory and register backend;
- PCM- or PWM-paced control-block scheduler selected without conflicting with
  the hardware PWM backend;
- atomic timeline compiler using GPIO set/clear masks and microsecond delays;
- one-shot, repeat, stop, busy, current-wave, and cancellation operations;
- bounded memory, pulse count, duration, and repeat count;
- underrun detection and emergency safe-state transition.

Gate: continuous CPU load does not violate the published timing envelope;
logic-analyzer results meet or beat pigpio on the same Pi 4 for the supported
wave corpus; daemon restart leaves no uncontrolled repeating output.

### M4 - pigpiod compatibility front end

Implement in test-driven groups:

1. GPIO modes, pull, read/write, bank operations, ticks, triggers, watchdogs.
2. Notifications, callbacks, glitch filters, and noise filters.
3. PWM, servo, hardware PWM, and hardware clock.
4. Wave create/delete/send/repeat/chain/status operations.
5. I2C/SMBus/I2C-Zip, SPI, and serial handles.
6. Bit-banged serial, I2C, and SPI.
7. Scripts and permitted administrative operations.

Gate: the support matrix is generated from tests, not maintained as an
unverified marketing table.  Existing `pigpio.py` applications connect without
source changes for every command marked compatible.

### M5 - Native features beyond pigpio

Deliverables:

- synchronized multi-pin transactions with absolute monotonic start times;
- scheduled remote execution that compensates for network latency;
- waveform upload deduplication by content hash;
- per-wave deadlines, leases, priorities, and cancellation;
- capture and replay with nanosecond timestamps where the backend supports it;
- structured diagnostics and Prometheus/OpenTelemetry export;
- optional redundant control connection without duplicate mutation execution.

Gate: each claimed advantage has a repeatable benchmark, failure-injection
test, and documented behavior under disconnect and daemon restart.

### M6 - Direct C compatibility

Deliverables:

- source-compatible C header and client shim;
- callback and notification threading semantics;
- optional ABI-compatible shared library where legally and technically viable;
- migration tests compiled from representative pigpio C applications.

Gate: the project may use the unqualified `pigpio-compatible` label only after
this milestone and the complete conformance suite pass.

## Native API shape

The existing newline-delimited JSON protocol remains supported as version 1.
Large waves and event streams use a version 2 framed protocol.

Representative native operations:

```text
hello(protocol_versions, requested_capabilities)
authenticate(key_id, proof)
pwm.start(pin, frequency_hz, duty_ppm, safe_level, lease_ms)
pwm.update(handle, frequency_hz?, duty_ppm?)
pwm.stop(handle)
wave.upload(metadata, content_hash, chunks)
wave.compile(upload_id)
wave.start(wave_id, mode, start_monotonic_ns?, lease_ms, safe_mask)
wave.cancel(execution_id)
wave.status(execution_id)
events.subscribe(pin_mask, edge_mask, filters)
resources.inspect()
```

Mutating responses include `request_id`, `outcome`, and the resulting handle or
execution ID.  Possible outcomes include `completed`, `rejected`, `cancelled`,
and `indeterminate`; a transport failure is never translated into an automatic
mutation retry.

## Performance targets

Targets must be finalized from measurements on the project's actual Pi 4.  The
first benchmark suite should nevertheless measure:

- GPIO sampling at 100 kHz, 200 kHz, 500 kHz, and 1 MHz;
- output-edge timing error at 1, 2, 5, and 10 microsecond pulse spacing;
- PWM frequency and duty error under idle and four-core CPU stress;
- wave start latency locally and over Tailscale;
- notification loss, ordering, and timestamp drift;
- reconnect, daemon crash, client crash, and network-partition outcomes;
- memory use and maximum sustainable pulse count.

Pi4gpio only claims that a target is met when the benchmark fixture, raw capture,
hardware/OS version, and comparison against the same-version pigpio build are
published together.

## Immediate implementation order

1. Add protocol version and capabilities without changing version 1 behavior.
2. Extract an internal command/event model from the current JSON dispatch.
3. Add a fake timing backend and deterministic scheduler tests on GitHub
   Actions.
4. Implement hardware PWM with a Pi-only integration test and safe shutdown.
5. Add the framed local protocol and waveform upload/compiler.
6. Implement the DMA timeline engine and logic-analyzer acceptance suite.
7. Add secure native TCP transport.
8. Add pigpio command translation in compatibility groups.

Remote control deliberately follows the local timing engine.  Network control
can invoke PWM and waves, but it must not be in the real-time scheduling loop.
