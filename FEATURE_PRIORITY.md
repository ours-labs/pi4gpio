# Feature foundations and future scope

The first implementation milestone established local GPIO, I2C, SPI, and UART
access behind one resource arbiter. For future scope,
[PIGPIO_SUPERSET_ROADMAP.md](PIGPIO_SUPERSET_ROADMAP.md) is authoritative.

## Implemented foundations

- I2C hardware access: `crates/pi4gpio-hw/src/i2c.rs`
- SPI hardware access: `crates/pi4gpio-hw/src/spi.rs`
- GPIO input/output and pull configuration: `crates/pi4gpio-hw/src/gpio.rs`
- UART access: `crates/pi4gpio-hw/src/uart.rs`
- timestamped GPIO edge monitoring: `crates/pi4gpio-hw/src/gpio_watch.rs`
- local Unix-socket daemon and resource ownership
- dependency-free Python client

Signal decoding remains a client responsibility. Availability claims are based
on source and automated tests; hardware performance claims require a published,
reproducible benchmark fixture.

## Future work

Hardware PWM, servo pulses, waveforms, remote access, and pigpiod compatibility
are planned milestones. Bit-banged buses, 1-Wire, and a script engine remain
uncommitted ideas until their behavior and maintenance cost are specified.

A roadmap feature is not described as available until its code, security
controls, tests, and public documentation are complete.
