# Enforcing exclusive hardware access

Pi4gpio arbitrates only clients that connect through `pi4gpiod`. It cannot
prevent another process from opening `/dev/gpiochip*`, `/dev/i2c-*`,
`/dev/spidev*`, or a UART directly.

## Deployment guidance

Before enabling a Pi4gpio-backed client:

1. Stop or isolate every process that can access the same hardware directly.
2. Confirm device ownership with `fuser` or an equivalent read-only inspection.
3. Apply service-level device restrictions such as `PrivateDevices=yes` and
   `DevicePolicy=closed` where they are compatible with the application.
4. Allow the client to access only the Pi4gpio Unix socket it needs.
5. Verify that the client has no physical-device file descriptors after startup.

Application-specific drop-ins, service names, host paths, and rollback records
belong in a private operations repository, not in this public source tree.

## Rollback to direct access

Rollback should be explicit. Automatic fallback from Pi4gpio to direct access
can duplicate an in-flight operation or overlap a still-running daemon.

1. Stop the Pi4gpio-backed client.
2. Confirm that its connection closed and the daemon released owned resources.
3. Confirm that no process holds the target devices.
4. Remove the application-specific Pi4gpio restriction.
5. Start the direct-mode client and verify that it is the sole hardware owner.

## Scope

Avoid changing shared udev permissions when per-service restrictions are
sufficient. Shared permission changes can affect unrelated applications on the
same Raspberry Pi.
