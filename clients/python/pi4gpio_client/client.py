"""Unix-socket client for pi4gpiod.

The client uses a newline-delimited JSON (NDJSON) protocol. The canonical
definition lives in ``crates/pi4gpio-daemon/src/protocol.rs``. Pi4gpio exposes
generic GPIO, I2C, SPI, and UART primitives; sensor-specific calibration and
decoding remain the caller's responsibility.

``BusRef`` is internally tagged with ``type`` while ``Operation`` uses serde's
external tagging. Unit variants are strings and variants with data are
single-key objects.
"""

from __future__ import annotations

import json
import socket
import threading
import time
from typing import Any, BinaryIO, Optional

DEFAULT_SOCKET_PATH = "/run/pi4gpio/pi4gpio.sock"

# Safety margin for daemon-side timeout and budget values.
_RESPONSE_TIMEOUT_MARGIN_SEC = 2.0


class Pi4gpioError(Exception):
    """Raised when pi4gpiod returns an error response (``ok: false``)."""


class Pi4gpioConnectionError(Pi4gpioError):
    """Raised when communication with pi4gpiod is interrupted.

    If ``reconnected`` is true, a new connection has been established. The
    in-flight request is never retried automatically because its execution
    status is unknown.
    """

    def __init__(self, message: str, *, reconnected: bool) -> None:
        super().__init__(message)
        self.reconnected = reconnected


class Pi4gpioClient:
    """Represent one connection to pi4gpiod.

    Bus locks are held per connection and released by ``*_release()`` or when
    the connection is closed. The client supports use as a context manager::

        with Pi4gpioClient() as client:
            client.gpio_write(pin=17, value=True)
    """

    def __init__(
        self,
        socket_path: str = DEFAULT_SOCKET_PATH,
        timeout: Optional[float] = 5.0,
        *,
        auto_reconnect: bool = True,
        reconnect_attempts: int = 8,
        reconnect_initial_delay: float = 0.1,
        reconnect_max_delay: float = 1.0,
    ):
        if reconnect_attempts < 1:
            raise ValueError("reconnect_attempts must be at least 1")
        if reconnect_initial_delay < 0 or reconnect_max_delay < 0:
            raise ValueError("reconnect delays must be non-negative")
        if reconnect_max_delay < reconnect_initial_delay:
            raise ValueError(
                "reconnect_max_delay must be at least reconnect_initial_delay"
            )
        self._socket_path = socket_path
        self._timeout = timeout
        self._auto_reconnect = auto_reconnect
        self._reconnect_attempts = reconnect_attempts
        self._reconnect_initial_delay = reconnect_initial_delay
        self._reconnect_max_delay = reconnect_max_delay
        self._sock: Optional[socket.socket] = None
        self._reader: Optional[BinaryIO] = None
        # Serialize NDJSON request/response pairs and connection state changes.
        self._request_lock = threading.RLock()

    def connect(self) -> "Pi4gpioClient":
        with self._request_lock:
            if self._sock is not None:
                return self
            self._connect_with_retries()
            return self

    def _create_connected_socket(self) -> socket.socket:
        """Create a connected socket; tests replace this boundary."""
        sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            sock.settimeout(self._timeout)
            sock.connect(self._socket_path)
            return sock
        except BaseException:
            sock.close()
            raise

    def _connect_once(self) -> None:
        sock = self._create_connected_socket()
        try:
            reader = sock.makefile("rb")
        except BaseException:
            sock.close()
            raise
        self._sock = sock
        self._reader = reader

    def _connect_with_retries(self) -> None:
        attempts = self._reconnect_attempts if self._auto_reconnect else 1
        delay = self._reconnect_initial_delay
        last_error: Optional[BaseException] = None
        for attempt in range(attempts):
            try:
                self._connect_once()
                return
            except (OSError, ValueError) as exc:
                last_error = exc
                if attempt + 1 < attempts:
                    time.sleep(delay)
                    delay = min(delay * 2, self._reconnect_max_delay)

        raise Pi4gpioConnectionError(
            f"could not connect to pi4gpiod after {attempts} attempts: {last_error}",
            reconnected=False,
        ) from last_error

    def _disconnect(self) -> None:
        # Detach shared state first so a failed close cannot reuse the socket.
        reader, self._reader = self._reader, None
        sock, self._sock = self._sock, None
        if reader is not None:
            try:
                reader.close()
            except OSError:
                pass
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass

    def close(self) -> None:
        with self._request_lock:
            self._disconnect()

    def __enter__(self) -> "Pi4gpioClient":
        return self.connect()

    def __exit__(self, exc_type: object, exc_val: object, exc_tb: object) -> bool:
        self.close()
        return False

    # --- Internal request/response handling ---

    def _request(
        self,
        bus: Optional[dict[str, Any]],
        op_name: str,
        op_args: Optional[dict[str, Any]] = None,
        min_response_timeout: Optional[float] = None,
    ) -> dict[str, Any]:
        """Send one operation and return its decoded response.

        ``op_name`` is the snake_case operation variant and ``op_args`` contains
        its fields. ``None`` encodes a unit variant as a bare string.
        ``min_response_timeout`` temporarily raises the socket timeout while
        waiting for an operation with its own daemon-side time budget.
        """
        with self._request_lock:
            if self._sock is None:
                self._connect_with_retries()
            assert self._sock is not None and self._reader is not None

            op: Any = op_name if op_args is None else {op_name: op_args}
            request: dict[str, Any] = {"op": op}
            if bus is not None:
                request["bus"] = bus
            payload = json.dumps(request, separators=(",", ":")) + "\n"

            request_sock = self._sock
            original_timeout = request_sock.gettimeout()
            needs_bump = (
                min_response_timeout is not None
                and original_timeout is not None
                and min_response_timeout > original_timeout
            )
            if needs_bump:
                request_sock.settimeout(min_response_timeout)
            try:
                request_sock.sendall(payload.encode("utf-8"))
                line = self._reader.readline()
                if not line:
                    raise EOFError("empty response")
                response: dict[str, Any] = json.loads(line)
            except (OSError, EOFError, json.JSONDecodeError, UnicodeDecodeError) as exc:
                self._disconnect()
                reconnected = False
                if self._auto_reconnect:
                    try:
                        self._connect_with_retries()
                        reconnected = True
                    except Pi4gpioConnectionError:
                        pass

                state = "reconnected" if reconnected else "reconnection failed"
                raise Pi4gpioConnectionError(
                    "communication with pi4gpiod was interrupted "
                    f"({state}); the in-flight request was not retried: {exc}",
                    reconnected=reconnected,
                ) from exc
            finally:
                # A failed request disconnects first, so do not touch that socket.
                if needs_bump and self._sock is request_sock:
                    try:
                        request_sock.settimeout(original_timeout)
                    except OSError:
                        pass

            if not response.get("ok", False):
                raise Pi4gpioError(response.get("error", "unknown error"))
            return response

    def protocol_info(
        self,
        protocol_versions: tuple[int, ...] = (1,),
        requested_capabilities: tuple[str, ...] = (),
    ) -> dict[str, Any]:
        """Negotiate a native protocol version and return daemon capabilities.

        This read-only request is bus-free and does not acquire hardware
        ownership. Existing v1 hardware request shapes are unchanged.
        """
        response = self._request(
            None,
            "hello",
            {
                "protocol_versions": list(protocol_versions),
                "requested_capabilities": list(requested_capabilities),
            },
        )
        protocol: dict[str, Any] = response["protocol"]
        return protocol

    # --- GPIO ---

    def gpio_read(self, pin: int, pull: str = "none") -> bool:
        """Set ``pull`` to ``"none"``, ``"up"``, or ``"down"``."""
        response = self._request(
            {"type": "gpio", "pin": pin}, "read", {"pull": pull}
        )
        return bool(response["value"])

    def gpio_write(self, pin: int, value: bool) -> bool:
        response = self._request(
            {"type": "gpio", "pin": pin}, "write", {"value": value}
        )
        return bool(response["value"])

    def gpio_watch_edges(
        self,
        pin: int,
        max_events: int,
        timeout_ms: int,
        pre_pulse_low_ms: Optional[int] = None,
        pull: str = "none",
    ) -> list[dict[str, Any]]:
        """Record timestamped edges through the GPIO v2 event interface.

        Returns ``[{"timestamp_ns": int, "rising": bool}, ...]``. Sensor-specific
        decoding remains the caller's responsibility. Set ``pull`` to
        ``"none"``, ``"up"``, or ``"down"`` as required by the circuit.
        """
        response = self._request(
            {"type": "gpio", "pin": pin},
            "watch_edges",
            {
                "pre_pulse_low_ms": pre_pulse_low_ms,
                "max_events": max_events,
                "timeout_ms": timeout_ms,
                "pull": pull,
            },
            min_response_timeout=timeout_ms / 1000 + _RESPONSE_TIMEOUT_MARGIN_SEC,
        )
        edges: list[dict[str, Any]] = response.get("edges") or []
        return edges

    def gpio_watch_edges_polled(
        self,
        pin: int,
        budget_ms: int,
        pre_pulse_low_ms: Optional[int] = None,
        pull: str = "none",
        idle_timeout_us: int = 300,
        glitch_filter_us: int = 0,
    ) -> list[dict[str, Any]]:
        """Record edges by polling raw GPIO levels in the daemon.

        This alternative to the GPIO v2 event interface is useful for signals
        whose transitions are not captured reliably by interrupt-driven
        sampling. Its return format matches ``gpio_watch_edges``. ``budget_ms``
        limits total polling time, ``idle_timeout_us`` ends capture after
        inactivity, and ``glitch_filter_us`` rejects shorter round-trip
        transitions. Keep the filter below the signal's shortest valid pulse.
        """
        response = self._request(
            {"type": "gpio", "pin": pin},
            "watch_edges_polled",
            {
                "pre_pulse_low_ms": pre_pulse_low_ms,
                "budget_ms": budget_ms,
                "idle_timeout_us": idle_timeout_us,
                "glitch_filter_us": glitch_filter_us,
                "pull": pull,
            },
            min_response_timeout=budget_ms / 1000 + _RESPONSE_TIMEOUT_MARGIN_SEC,
        )
        edges: list[dict[str, Any]] = response.get("edges") or []
        return edges

    def gpio_release(self, pin: int) -> None:
        self._request({"type": "gpio", "pin": pin}, "release")

    # --- I2C ---

    def i2c_read(self, bus: int, addr: int, length: int) -> bytes:
        response = self._request(
            {"type": "i2c", "bus": bus, "addr": addr},
            "read_bytes",
            {"length": length},
        )
        return bytes(response.get("bytes") or [])

    def i2c_write(self, bus: int, addr: int, data: bytes) -> None:
        self._request(
            {"type": "i2c", "bus": bus, "addr": addr},
            "write_bytes",
            {"data": list(data)},
        )

    def i2c_write_read(self, bus: int, addr: int, data: bytes, length: int) -> bytes:
        response = self._request(
            {"type": "i2c", "bus": bus, "addr": addr},
            "write_read_bytes",
            {"data": list(data), "length": length},
        )
        return bytes(response.get("bytes") or [])

    def i2c_release(self, bus: int) -> None:
        # Locks are keyed by bus, so the address is ignored for release.
        self._request({"type": "i2c", "bus": bus, "addr": 0}, "release")

    # --- SPI ---

    def spi_transfer(self, bus: int, chip_select: int, data: bytes) -> bytes:
        response = self._request(
            {"type": "spi", "bus": bus, "chip_select": chip_select},
            "transfer",
            {"data": list(data)},
        )
        return bytes(response.get("bytes") or [])

    def spi_release(self, bus: int, chip_select: int) -> None:
        self._request(
            {"type": "spi", "bus": bus, "chip_select": chip_select}, "release"
        )

    # --- UART ---

    def uart_read(self, port: int, baud_rate: int, length: int) -> bytes:
        response = self._request(
            {"type": "uart", "port": port, "baud_rate": baud_rate},
            "read_bytes",
            {"length": length},
        )
        return bytes(response.get("bytes") or [])

    def uart_write(self, port: int, baud_rate: int, data: bytes) -> None:
        self._request(
            {"type": "uart", "port": port, "baud_rate": baud_rate},
            "write_bytes",
            {"data": list(data)},
        )

    def uart_release(self, port: int, baud_rate: int) -> None:
        self._request(
            {"type": "uart", "port": port, "baud_rate": baud_rate}, "release"
        )
