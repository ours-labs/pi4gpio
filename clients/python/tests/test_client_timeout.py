"""Regression tests for ``Pi4gpioClient`` socket timeout handling.

The tests use ``socket.socketpair()`` to verify that daemon-side operation
budgets can exceed the client's normal socket timeout without requiring a live
Unix-socket daemon.
"""

import json
import socket
import threading
import time
import unittest

from pi4gpio_client.client import Pi4gpioClient


def _make_client_with_socketpair(base_timeout):
    """Create a client wired directly to one end of a socket pair."""
    client_sock, server_sock = socket.socketpair()
    client_sock.settimeout(base_timeout)

    client = Pi4gpioClient(
        socket_path="<socketpair>", timeout=base_timeout, reconnect_attempts=1
    )
    client._sock = client_sock
    client._reader = client_sock.makefile("rb")
    return client, server_sock


def _serve_one_request(server_sock, delay_sec, response):
    """Receive one request and respond after ``delay_sec`` seconds."""
    reader = server_sock.makefile("rb")
    reader.readline()  # The request body is irrelevant to timeout behavior.
    time.sleep(delay_sec)
    payload = json.dumps(response).encode("utf-8") + b"\n"
    server_sock.sendall(payload)


def _capture_one_request(server_sock, captured):
    reader = server_sock.makefile("rb")
    captured.append(json.loads(reader.readline()))
    server_sock.sendall(b'{"ok":true,"edges":[]}\n')


def _capture_protocol_request(server_sock, captured):
    reader = server_sock.makefile("rb")
    captured.append(json.loads(reader.readline()))
    server_sock.sendall(
        b'{"ok":true,"protocol":{"selected_version":1,'
        b'"supported_versions":[1],"available_capabilities":["gpio.read"],'
        b'"enabled_capabilities":["gpio.read"],"unavailable_capabilities":[],'
        b'"clock":{"source":"CLOCK_MONOTONIC","unit":"nanoseconds",'
        b'"epoch":"unspecified_per_boot"}}}\n'
    )


class ClientTimeoutTest(unittest.TestCase):
    def test_protocol_info_is_bus_free_and_preserves_v1_transport(self):
        client, server_sock = _make_client_with_socketpair(base_timeout=1.0)
        captured = []
        try:
            server_thread = threading.Thread(
                target=_capture_protocol_request,
                args=(server_sock, captured),
            )
            server_thread.start()
            info = client.protocol_info(
                protocol_versions=(1, 2),
                requested_capabilities=("gpio.read",),
            )
            server_thread.join(timeout=2)

            self.assertNotIn("bus", captured[0])
            self.assertEqual(captured[0]["op"]["hello"]["protocol_versions"], [1, 2])
            self.assertEqual(info["selected_version"], 1)
        finally:
            client.close()
            server_sock.close()

    def test_long_watch_edges_survives_short_base_timeout(self):
        """An operation budget may safely exceed the base socket timeout."""
        client, server_sock = _make_client_with_socketpair(base_timeout=1.0)
        try:
            server_thread = threading.Thread(
                target=_serve_one_request,
                args=(server_sock, 2.0, {"ok": True, "edges": []}),
            )
            server_thread.start()

            edges = client.gpio_watch_edges(
                pin=26, max_events=90, timeout_ms=8000, pre_pulse_low_ms=18, pull="up"
            )
            server_thread.join(timeout=5)

            self.assertEqual(edges, [])
        finally:
            client.close()
            server_sock.close()

    def test_timeout_restored_after_request(self):
        """The base socket timeout is restored after a response."""
        client, server_sock = _make_client_with_socketpair(base_timeout=1.0)
        try:
            server_thread = threading.Thread(
                target=_serve_one_request,
                args=(server_sock, 0.1, {"ok": True, "edges": []}),
            )
            server_thread.start()

            client.gpio_watch_edges(
                pin=26, max_events=90, timeout_ms=8000, pre_pulse_low_ms=18, pull="up"
            )
            server_thread.join(timeout=5)

            self.assertEqual(client._sock.gettimeout(), 1.0)
        finally:
            client.close()
            server_sock.close()

    def test_short_op_unaffected_by_bump_logic(self):
        """Operations without a daemon-side timeout keep the base timeout."""
        client, server_sock = _make_client_with_socketpair(base_timeout=5.0)
        try:
            server_thread = threading.Thread(
                target=_serve_one_request,
                args=(server_sock, 0.05, {"ok": True, "value": True}),
            )
            server_thread.start()

            value = client.gpio_read(pin=17, pull="up")
            server_thread.join(timeout=5)

            self.assertTrue(value)
            self.assertEqual(client._sock.gettimeout(), 5.0)
        finally:
            client.close()
            server_sock.close()

    def test_polled_watch_sends_configurable_idle_timeout(self):
        client, server_sock = _make_client_with_socketpair(base_timeout=1.0)
        captured = []
        try:
            server_thread = threading.Thread(
                target=_capture_one_request,
                args=(server_sock, captured),
            )
            server_thread.start()
            client.gpio_watch_edges_polled(
                pin=17,
                budget_ms=25,
                idle_timeout_us=2500,
                glitch_filter_us=10,
            )
            server_thread.join(timeout=2)

            args = captured[0]["op"]["watch_edges_polled"]
            self.assertEqual(args["idle_timeout_us"], 2500)
            self.assertEqual(args["glitch_filter_us"], 10)
        finally:
            client.close()
            server_sock.close()


if __name__ == "__main__":
    unittest.main()
