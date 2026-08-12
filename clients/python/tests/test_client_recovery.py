"""デーモン停止・再起動時のクライアント復旧に関する障害注入テスト。"""

import collections
import json
import socket
import threading
import unittest

from pi4gpio_client import Pi4gpioClient, Pi4gpioConnectionError


class _SocketSequenceClient(Pi4gpioClient):
    """接続試行ごとに、準備済みソケットまたは例外を返すテスト用クライアント。"""

    def __init__(self, sequence):
        super().__init__(
            socket_path="<fault-injection>",
            timeout=0.5,
            reconnect_attempts=4,
            reconnect_initial_delay=0,
            reconnect_max_delay=0,
        )
        self._sequence = collections.deque(sequence)
        self.connection_attempts = 0

    def _create_connected_socket(self):
        self.connection_attempts += 1
        if not self._sequence:
            raise ConnectionRefusedError("no injected daemon")
        item = self._sequence.popleft()
        if isinstance(item, BaseException):
            raise item
        item.settimeout(self._timeout)
        return item


def _drop_after_one_request(server_sock):
    """要求を受信した直後に応答せず接続を切り、デーモン異常終了を模擬する。"""
    with server_sock:
        reader = server_sock.makefile("rb")
        reader.readline()
        reader.close()


def _serve_one_success(server_sock, response):
    with server_sock:
        reader = server_sock.makefile("rb")
        reader.readline()
        server_sock.sendall(json.dumps(response).encode("utf-8") + b"\n")
        reader.close()


class ClientRecoveryFaultInjectionTest(unittest.TestCase):
    def test_disconnect_reconnects_but_does_not_replay_in_flight_request(self):
        old_client, old_server = socket.socketpair()
        new_client, new_server = socket.socketpair()
        client = _SocketSequenceClient([new_client])
        client._sock = old_client
        client._reader = old_client.makefile("rb")

        dropper = threading.Thread(
            target=_drop_after_one_request, args=(old_server,), daemon=True
        )
        dropper.start()
        try:
            with self.assertRaises(Pi4gpioConnectionError) as caught:
                client.gpio_write(pin=17, value=True)
            dropper.join(timeout=2)

            self.assertTrue(caught.exception.reconnected)
            self.assertEqual(client.connection_attempts, 1)

            # 最初の要求がdaemon側で実行済みかは判別不能である。再接続先へ
            # 自動再送されていないことを、受信タイムアウトで直接確認する。
            new_server.settimeout(0.05)
            with self.assertRaises((TimeoutError, socket.timeout)):
                new_server.recv(1)

            responder = threading.Thread(
                target=_serve_one_success,
                args=(new_server, {"ok": True, "value": False}),
                daemon=True,
            )
            responder.start()
            self.assertFalse(client.gpio_read(pin=17))
            responder.join(timeout=2)
        finally:
            client.close()
            new_server.close()

    def test_reconnect_retries_until_restarted_daemon_is_available(self):
        old_client, old_server = socket.socketpair()
        restarted_client, restarted_server = socket.socketpair()
        client = _SocketSequenceClient(
            [
                ConnectionRefusedError("daemon still down"),
                ConnectionRefusedError("socket not recreated yet"),
                restarted_client,
            ]
        )
        client._sock = old_client
        client._reader = old_client.makefile("rb")

        dropper = threading.Thread(
            target=_drop_after_one_request, args=(old_server,), daemon=True
        )
        dropper.start()
        try:
            with self.assertRaises(Pi4gpioConnectionError) as caught:
                client.gpio_read(pin=6, pull="up")
            self.assertTrue(caught.exception.reconnected)
            self.assertEqual(client.connection_attempts, 3)
        finally:
            dropper.join(timeout=2)
            client.close()
            restarted_server.close()

    def test_initial_connection_failure_is_bounded_and_typed(self):
        client = _SocketSequenceClient(
            [ConnectionRefusedError("down") for _ in range(4)]
        )
        try:
            with self.assertRaises(Pi4gpioConnectionError) as caught:
                client.connect()
            self.assertFalse(caught.exception.reconnected)
            self.assertEqual(client.connection_attempts, 4)
        finally:
            client.close()


if __name__ == "__main__":
    unittest.main()
