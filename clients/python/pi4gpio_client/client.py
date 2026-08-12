"""pi4gpiodへのUnixソケット経由クライアント。

改行区切りJSON（NDJSON）プロトコルでpi4gpiodと通信する。プロトコル定義は
pi4gpioリポジトリの`crates/pi4gpio-daemon/src/protocol.rs`が正本。

BME280のキャリブレーション計算・DHT22の40ビットデコードのようなセンサー
固有のロジックはこのクライアントの責務ではない。pi4gpiodは汎用バス
プリミティブ（GPIO/I2C/SPI/UARTの生の読み書き）のみを提供する設計で、
センサー固有の解釈は呼び出し側に残る。

`bus`（`BusRef`）と`op`（`Operation`）はserdeでのタグ付け方式が異なる点に
注意。`BusRef`は`#[serde(tag = "type")]`で内部タグ付き
（例: ``{"type": "gpio", "pin": 17}``）だが、`Operation`にはタグ属性が無く
serdeのデフォルト（外部タグ付き）になる。データを持たないバリアント
（`Release`）は裸の文字列（例: ``"release"``）、データを持つバリアント
は1キーのオブジェクト（例: ``{"write": {"value": true}}``）で表現される。
"""

from __future__ import annotations

import json
import socket
import threading
import time
from typing import Any, BinaryIO, Optional

DEFAULT_SOCKET_PATH = "/run/pi4gpio/pi4gpio.sock"

# daemon側の待ち時間（timeout_ms/budget_ms）に対してソケットタイムアウトを
# 引き上げる際の安全マージン。ネットワーク・プロセス間のオーバーヘッド分。
_RESPONSE_TIMEOUT_MARGIN_SEC = 2.0


class Pi4gpioError(Exception):
    """pi4gpiodがエラーレスポンス（``ok: false``）を返した場合に送出する。"""


class Pi4gpioConnectionError(Pi4gpioError):
    """pi4gpiodとの通信が切断された場合に送出する。

    ``reconnected``が真なら、新しい接続の確立までは完了している。ただし、
    切断時に処理中だった要求は二重実行を避けるため自動再送しない。呼び出し側は
    この例外をその周期の失敗として扱い、次の通常周期で操作を再実行する。
    """

    def __init__(self, message: str, *, reconnected: bool) -> None:
        super().__init__(message)
        self.reconnected = reconnected


class Pi4gpioClient:
    """pi4gpiodへの1接続を表すクライアント。

    バスのロックは接続単位で保持される（サーバー側の`LockTable`参照）ため、
    同じ接続を使い回している限り、確保したバスは他クライアントの割り込み
    から守られる。明示的に`*_release()`を呼ぶか、接続を閉じる（`close()`
    または`with`ブロックを抜ける）とロックが解放される。

    with文での利用を想定している::

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
            raise ValueError("reconnect_attemptsは1以上である必要があります")
        if reconnect_initial_delay < 0 or reconnect_max_delay < 0:
            raise ValueError("再接続待ち時間は0以上である必要があります")
        if reconnect_max_delay < reconnect_initial_delay:
            raise ValueError(
                "reconnect_max_delayはreconnect_initial_delay以上である必要があります"
            )
        self._socket_path = socket_path
        self._timeout = timeout
        self._auto_reconnect = auto_reconnect
        self._reconnect_attempts = reconnect_attempts
        self._reconnect_initial_delay = reconnect_initial_delay
        self._reconnect_max_delay = reconnect_max_delay
        self._sock: Optional[socket.socket] = None
        self._reader: Optional[BinaryIO] = None
        # 1接続のNDJSON要求/応答は直列である。再接続中に別スレッドが同じ
        # ソケットを使わないよう、接続状態の変更も同じロックで保護する。
        self._request_lock = threading.RLock()

    def connect(self) -> "Pi4gpioClient":
        with self._request_lock:
            if self._sock is not None:
                return self
            self._connect_with_retries()
            return self

    def _create_connected_socket(self) -> socket.socket:
        """接続済みソケットを作る。テストではこの境界だけを差し替える。"""
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
            f"pi4gpiodへ接続できませんでした（{attempts}回試行）: {last_error}",
            reconnected=False,
        ) from last_error

    def _disconnect(self) -> None:
        # 先に共有状態から外す。close中に例外が出ても壊れた接続を再利用しない。
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

    # --- 内部: リクエスト送受信 ---

    def _request(
        self,
        bus: dict[str, Any],
        op_name: str,
        op_args: Optional[dict[str, Any]] = None,
        min_response_timeout: Optional[float] = None,
    ) -> dict[str, Any]:
        """`op_name`は`Operation`のバリアント名（snake_case）、`op_args`は
        そのバリアントが持つフィールド。`op_args`が`None`なら
        データを持たないバリアント（`Read`/`Release`）として裸の文字列で
        送る。

        `min_response_timeout`: このリクエストの応答を待つ間だけ、ソケット
        のタイムアウトを最低でもこの秒数まで一時的に引き上げる（応答後は
        元の値に戻す）。`gpio_watch_edges`/`gpio_watch_edges_polled`のように
        呼び出し側がdaemon側の待ち時間（`timeout_ms`/`budget_ms`）を独自に
        指定できる操作では、それがクライアント自身のソケットタイムアウト
        （デフォルト5秒）を超えると、daemonが応答するより先にクライアント
        側がタイムアウトしてしまうことがある（実機検証で発見）。
        """
        with self._request_lock:
            if self._sock is None:
                self._connect_with_retries()
            assert self._sock is not None and self._reader is not None

            op: Any = op_name if op_args is None else {op_name: op_args}
            payload = (
                json.dumps({"bus": bus, "op": op}, separators=(",", ":")) + "\n"
            )

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
                    raise EOFError("空の応答")
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

                state = "再接続済み" if reconnected else "再接続失敗"
                raise Pi4gpioConnectionError(
                    "pi4gpiodとの通信が切断されました"
                    f"（{state}）。処理中の要求は安全のため自動再送していません: {exc}",
                    reconnected=reconnected,
                ) from exc
            finally:
                # 障害時は_disconnect()済みなので、閉じたソケットへ触れない。
                if needs_bump and self._sock is request_sock:
                    try:
                        request_sock.settimeout(original_timeout)
                    except OSError:
                        pass

            if not response.get("ok", False):
                raise Pi4gpioError(response.get("error", "不明なエラー"))
            return response

    # --- GPIO ---

    def gpio_read(self, pin: int, pull: str = "none") -> bool:
        """`pull`は``"none"``/``"up"``/``"down"``のいずれか。"""
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
        """エッジをタイムスタンプ付きで記録する（Tier 2）。

        戻り値は``[{"timestamp_ns": int, "rising": bool}, ...]``。DHT22の
        40ビットデコード等、センサー固有の解釈は呼び出し側の責務。

        `pull`は``"none"``/``"up"``/``"down"``のいずれか。DHT22モジュールに
        外部プルアップが無い場合は``"up"``を指定する。
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
        """`gpio_watch_edges`（カーネルのGPIO v2エッジ割り込み、Tier 2）の
        代替。実機検証で、DHT22のような電圧遷移が緩やかなプロトコルでは
        割り込みが一部の遷移を取りこぼすことがあると判明したため
        場合があるため、`/dev/gpiomem`の生レベルを
        daemon側で高速busy-loopポーリングし、レベル変化をエッジとして
        記録する（Tier 1相当）。戻り値の形式は`gpio_watch_edges`と同一
        （``[{"timestamp_ns": int, "rising": bool}, ...]``）なので、
        呼び出し側のデコードロジックはどちらを使っても変更不要。

        `budget_ms`は最大ポーリング時間、`idle_timeout_us`は最後の遷移から
        通信終了とみなすまでの無変化時間。`glitch_filter_us`を指定すると、
        それより短く元のレベルへ戻る変化を除外する。対象信号の仕様が保証する
        最短パルスより短い値だけを指定する。
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
        # ロックはbus単位で管理されaddrは無視されるため（protocol.rsの
        # From<&BusRef> for BusId参照）、addrはダミー値でよい。
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
