"""Regression tests for `Pi4gpioClient` socket timeout handling.

The mock server uses `socket.socketpair()` to verify that long edge-watch
operations can extend the response timeout without requiring a running daemon.
"""

import json
import socket
import threading
import time
import unittest

from pi4gpio_client.client import Pi4gpioClient


def _make_client_with_socketpair(base_timeout):
    """`connect()`（実ソケットパスへの接続）を経由せず、`socketpair()`の
    片方を直接差し込んだクライアントを作る。テスト専用の配線であり、
    本番コードにテスト用の分岐を追加せずに済む。
    """
    client_sock, server_sock = socket.socketpair()
    client_sock.settimeout(base_timeout)

    client = Pi4gpioClient(
        socket_path="<socketpair>", timeout=base_timeout, reconnect_attempts=1
    )
    client._sock = client_sock
    client._reader = client_sock.makefile("rb")
    return client, server_sock


def _serve_one_request(server_sock, delay_sec, response):
    """1リクエストだけ受けて、`delay_sec`秒待ってから応答を返す。"""
    reader = server_sock.makefile("rb")
    reader.readline()  # リクエスト本体は検証しない(タイムアウト挙動のみが関心事)
    time.sleep(delay_sec)
    payload = json.dumps(response).encode("utf-8") + b"\n"
    server_sock.sendall(payload)


def _capture_one_request(server_sock, captured):
    reader = server_sock.makefile("rb")
    captured.append(json.loads(reader.readline()))
    server_sock.sendall(b'{"ok":true,"edges":[]}\n')


class ClientTimeoutTest(unittest.TestCase):
    def test_long_watch_edges_survives_short_base_timeout(self):
        """base_timeout(1秒)より応答が遅く(2秒)ても、timeout_ms由来の
        min_response_timeoutが優先され、タイムアウトせず応答を受け取れる。
        """
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
        """引き上げたソケットタイムアウトは、応答後に元の値へ戻る。"""
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
        """`timeout_ms`を伴わない通常操作は、従来通りbase_timeoutのまま。"""
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
