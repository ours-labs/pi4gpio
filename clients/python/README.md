# pi4gpio-client

A dependency-free Python client for the local Unix-socket API exposed by [Pi4gpio](https://github.com/ours-labs/pi4gpio).

```python
from pi4gpio_client import Pi4gpioClient

with Pi4gpioClient() as client:
    client.gpio_write(pin=17, value=True)
    level = client.gpio_read(pin=17)

    chip_id = client.i2c_write_read(
        bus=1, addr=0x76, data=bytes([0xD0]), length=1
    )
    adc = client.spi_transfer(
        bus=0, chip_select=0, data=bytes([0x06, 0x00, 0x00])
    )

    client.uart_write(
        port=0,
        baud_rate=9600,
        data=b"\xff\x01\x86\x00\x00\x00\x00\x00\x79",
    )
    response = client.uart_read(port=0, baud_rate=9600, length=9)

    edges = client.gpio_watch_edges_polled(
        pin=17,
        budget_ms=20,
        idle_timeout_us=2_000,
    )
```

Sensor-specific calibration, decoding, and validation are intentionally outside this package. The daemon and client expose generic GPIO, I2C, SPI, and UART operations.

## Reconnection behavior

After a disconnect, the client discards the broken socket and reconnects with bounded exponential backoff. It never replays the in-flight request because the operation may already have completed on the daemon.

```python
from pi4gpio_client import Pi4gpioClient, Pi4gpioConnectionError

client = Pi4gpioClient(reconnect_attempts=8)
try:
    value = client.gpio_read(pin=17)
except Pi4gpioConnectionError as exc:
    # exc.reconnected reports whether the transport recovered.
    # The interrupted operation was not replayed.
    record_failed_sample(str(exc))
```

Set `auto_reconnect=False` to disable automatic reconnection. After reconnecting, the next ordinary operation acquires resources under the new connection.

The daemon rejects bus/operation mismatches and excessive transfers or waits before acquiring a resource lock.

## Development install

```bash
pip install -e .
```
