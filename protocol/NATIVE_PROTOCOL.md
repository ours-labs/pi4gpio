# Native protocol versioning

Pi4gpio's existing newline-delimited JSON request and response format is native
protocol version 1. Existing hardware requests retain their original shape.

## Capability negotiation

A client may send a bus-free, read-only request before using hardware:

```json
{"op":{"hello":{"protocol_versions":[1],"requested_capabilities":[]}}}
```

The daemon selects the highest mutually supported version. An empty capability
list requests every available capability. A non-empty list produces separate
`enabled_capabilities` and `unavailable_capabilities` arrays. Negotiation never
acquires a GPIO, I2C, SPI, or UART resource.

If no protocol version overlaps, the response has `ok: false`, the error
`unsupported_protocol_version`, and protocol metadata with a null
`selected_version`. This lets a client report a precise compatibility failure.

## Clock contract

Version 1 event timestamps use Linux `CLOCK_MONOTONIC` in nanoseconds. The epoch
is unspecified and restarts on boot, so timestamps are comparable only within
one boot. They must not be treated as UTC or persisted as wall-clock time
without an explicitly recorded conversion sample.

## Compatibility claims

`pigpio-command-support.json` records implementation and conformance evidence
for pigpiod commands. `not_implemented` means no compatible command path exists,
`implemented` means a command path exists, and `conformant` is reserved for
commands whose behavior and error cases pass the comparison harness. The
current matrix makes no pigpiod compatibility claim.
