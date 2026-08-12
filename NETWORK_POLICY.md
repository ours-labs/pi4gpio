# Network policy

## Current implementation

Pi4gpio 0.1.2 is local-only. `pi4gpiod` listens on a Unix-domain socket and does not implement a TCP listener, Tailscale binding, API-key authentication, or remote-administration commands.

Do not expose the Unix socket through a TCP proxy, port forward, tunnel, or public relay. The local protocol was not designed as a network security boundary.

## Accepted design for future remote access

Remote control is an approved roadmap goal, not a current capability. Any implementation must satisfy all of the following before it is enabled:

1. Bind only to a private Tailscale address or another explicitly configured private interface.
2. Require application-layer authentication in addition to network membership.
3. Store credentials outside the repository and support rotation and revocation.
4. Apply least-privilege authorization by operation and hardware resource.
5. Use bounded requests, rate limits, audit logging, and fail-closed defaults.
6. Keep remote administration separate from ordinary hardware operations.
7. Include threat-model, negative, restart, and credential-revocation tests.

mTLS may be reconsidered if the deployment grows beyond individually managed systems. It is not required for the first private-network implementation, but transport confidentiality and peer authentication remain mandatory.

## Distribution model

Each operator is responsible for their own device, credentials, and network policy. The project must not ship shared keys, default secrets, or a centrally operated public control endpoint.

The detailed implementation sequence is maintained in [PIGPIO_SUPERSET_ROADMAP.md](PIGPIO_SUPERSET_ROADMAP.md), and architectural commitments are recorded in [docs/DECISIONS.md](docs/DECISIONS.md).
