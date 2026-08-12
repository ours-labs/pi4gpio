# Security model

## Trust boundary

Pi4gpio 0.1.2 accepts local Unix-domain socket connections only. Local peer credentials identify the connecting process. The daemon does not provide network authentication or authorization.

## Deployment requirements

- Run the daemon as a dedicated unprivileged service account.
- Grant that account only the device-group membership required by the enabled interfaces.
- Restrict the socket directory to the intended client group.
- Prevent client services from opening GPIO, I2C, SPI, or UART devices directly when exclusive mediation is required.
- Keep secrets and environment-specific host, path, and account values outside the repository.

## Out of scope

The current release does not defend against a privileged local process, a process with independent device permissions, a compromised kernel, physical tampering, or a network bridge that exposes the local socket.

## Remote transports

Tailscale or another private network may protect routing, but routing alone is not application authentication. Any future remote mode must define mutual authentication, least-privilege authorization, replay resistance, request limits, revocation, audit events, and a disabled-by-default migration path before a network listener is released.
