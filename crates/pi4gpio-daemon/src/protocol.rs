//! ワイヤープロトコル（雛形段階）。
//!
//! 改行区切りJSON（1行1リクエスト/1行1レスポンス）。バイナリ化・多重化などの
//! 最適化は、Tier 1操作が実装されパフォーマンス要件が明確になってから検討する
//! （現段階ではPythonクライアント側での可読性・実装のしやすさを優先）。
//!
//! `Operation`はGPIO用（`Read`/`Write`、1ビット単位）、I2C/UART用
//! （`ReadBytes`/`WriteBytes`、方向が別々のバイト列。I2Cはさらに結合
//! トランザクション`WriteReadBytes`を持つ）、SPI用（`Transfer`、送信と
//! 同時に同じ長さを受信する全二重転送）に分かれる。バスの種類に合わない
//! 操作が来た場合は`socket.rs`の`dispatch`が`malformed`で拒否する。
//!
//! いずれの操作もバスを暗黙に確保する（未確保なら`LockTable::try_acquire`）。
//! I2Cはバス単位でロックする（`addr`単位ではない）——同じバス上の別デバイス
//! への割り込みも防ぐのが目的。確保したバスは`Release`または
//! 切断（`socket.rs`の接続ハンドラ側で処理）までそのクライアントが保持する。

use crate::lock::BusId;
use serde::{Deserialize, Serialize};

pub const DEFAULT_POLL_IDLE_TIMEOUT_US: u64 = 300;
const MAX_TRANSFER_BYTES: usize = 1_048_576;
const MAX_EDGE_EVENTS: usize = 1_000_000;
const MAX_OPERATION_MS: u64 = 60_000;
const MAX_POLL_IDLE_TIMEOUT_US: u64 = 60_000_000;

fn default_poll_idle_timeout_us() -> u64 {
    DEFAULT_POLL_IDLE_TIMEOUT_US
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub bus: BusRef,
    pub op: Operation,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BusRef {
    Gpio {
        pin: u32,
    },
    I2c {
        bus: u8,
        addr: u8,
    },
    Spi {
        bus: u8,
        chip_select: u8,
    },
    /// `port`は`/dev/ttyS{port}`に対応する（daemon側の命名規約、
    /// `socket.rs`参照）。`baud_rate`はそのロック保持期間の初回オープン時に有効。
    /// Releaseまたは切断時にポートをcloseし、次の所有者は指定値で開き直す。
    Uart {
        port: u8,
        baud_rate: u32,
    },
}

impl From<&BusRef> for BusId {
    fn from(bus: &BusRef) -> Self {
        match *bus {
            BusRef::Gpio { pin } => BusId::Gpio(pin),
            // addrはロック粒度に含めない。同じバスの別アドレスへのアクセスも
            // トランザクション途中の割り込みから守るため、バス全体を排他する。
            BusRef::I2c { bus, .. } => BusId::I2c(bus),
            BusRef::Spi { bus, chip_select } => BusId::Spi(bus, chip_select),
            BusRef::Uart { port, .. } => BusId::Uart(port),
        }
    }
}

/// GPIO入力のプルアップ/ダウン設定。`pi4gpio_hw::gpio::PullMode`のワイヤー版。
#[derive(Debug, Deserialize, Clone, Copy, Default)]
#[serde(rename_all = "snake_case")]
pub enum PullWire {
    #[default]
    None,
    Up,
    Down,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    // GPIO用: 1ビット単位。`pull`省略時はNone（フィールドを省略できる
    // クライアントとの互換のため`#[serde(default)]`）。
    Read {
        #[serde(default)]
        pull: PullWire,
    },
    Write {
        value: bool,
    },
    // I2C用: 方向が別々のバイト列（将来UARTでも流用予定）。
    ReadBytes {
        length: usize,
    },
    WriteBytes {
        data: Vec<u8>,
    },
    WriteReadBytes {
        data: Vec<u8>,
        length: usize,
    },
    // SPI用: 送信と同時に同じ長さを受信する全二重転送。
    Transfer {
        data: Vec<u8>,
    },
    // GPIO用（Tier 2）: エッジをタイムスタンプ付きで記録する。
    // `pre_pulse_low_ms`を指定すると、監視開始前にそのピンをLOWに駆動して
    // から`Some(ms)`ミリ秒待つ（DHT22等のスタート信号パターン）。`pull`は
    // 監視中のSoC側プルアップ/ダウン（外部プルアップが無い回路向け、
    // `pull`省略時はNone）。
    WatchEdges {
        pre_pulse_low_ms: Option<u64>,
        max_events: usize,
        timeout_ms: u64,
        #[serde(default)]
        pull: PullWire,
    },
    // GPIO用（Tier 1高速ポーリング版）: `/dev/gpiomem`の生レベルを
    // busy-loopで連続サンプリングし、レベル変化をエッジとして記録する。
    // カーネルGPIO v2の割り込みでは拾えない信号にも使える一方、CPUを占有し、
    // ポーリング精度がタイムスタンプ粒度になる。`budget_ms`到達、または
    // 最後の遷移から`idle_timeout_us`経過のいずれか早い方で打ち切る。
    WatchEdgesPolled {
        pre_pulse_low_ms: Option<u64>,
        budget_ms: u64,
        /// 最後の遷移後、通信終了とみなすまでの無変化時間。省略時の300usは
        /// 既存クライアントとの後方互換値であり、センサー固有要件では上書きする。
        #[serde(default = "default_poll_idle_timeout_us")]
        idle_timeout_us: u64,
        /// この時間より短く元のレベルへ戻る変化をグリッチとして除外する。
        /// 0（既定）は無効。対象信号の仕様が保証する最短パルスより短くする。
        #[serde(default)]
        glitch_filter_us: u64,
        #[serde(default)]
        pull: PullWire,
    },
    Release,
}

impl Request {
    /// ハードウェアを確保する前に、操作種別と資源消費量を検証する。
    pub fn validate(&self) -> Result<(), String> {
        let compatible = matches!(
            (&self.bus, &self.op),
            (_, Operation::Release)
                | (BusRef::Gpio { .. }, Operation::Read { .. })
                | (BusRef::Gpio { .. }, Operation::Write { .. })
                | (BusRef::Gpio { .. }, Operation::WatchEdges { .. })
                | (BusRef::Gpio { .. }, Operation::WatchEdgesPolled { .. })
                | (BusRef::I2c { .. }, Operation::ReadBytes { .. })
                | (BusRef::I2c { .. }, Operation::WriteBytes { .. })
                | (BusRef::I2c { .. }, Operation::WriteReadBytes { .. })
                | (BusRef::Spi { .. }, Operation::Transfer { .. })
                | (BusRef::Uart { .. }, Operation::ReadBytes { .. })
                | (BusRef::Uart { .. }, Operation::WriteBytes { .. })
        );
        if !compatible {
            return Err("指定バスではこの操作を使用できません".to_string());
        }

        let validate_length = |name: &str, length: usize| {
            if length == 0 || length > MAX_TRANSFER_BYTES {
                Err(format!(
                    "{name}は1..={MAX_TRANSFER_BYTES} bytesで指定してください"
                ))
            } else {
                Ok(())
            }
        };
        let validate_pre_pulse = |value: Option<u64>| {
            if value.is_some_and(|ms| ms > MAX_OPERATION_MS) {
                Err(format!(
                    "pre_pulse_low_msは0..={MAX_OPERATION_MS}で指定してください"
                ))
            } else {
                Ok(())
            }
        };

        match &self.op {
            Operation::ReadBytes { length } => validate_length("length", *length),
            Operation::WriteBytes { data } | Operation::Transfer { data } => {
                validate_length("data長", data.len())
            }
            Operation::WriteReadBytes { data, length } => {
                validate_length("data長", data.len())?;
                validate_length("length", *length)
            }
            Operation::WatchEdges {
                pre_pulse_low_ms,
                max_events,
                timeout_ms,
                ..
            } => {
                validate_pre_pulse(*pre_pulse_low_ms)?;
                if *max_events == 0 || *max_events > MAX_EDGE_EVENTS {
                    return Err(format!(
                        "max_eventsは1..={MAX_EDGE_EVENTS}で指定してください"
                    ));
                }
                if *timeout_ms == 0 || *timeout_ms > MAX_OPERATION_MS {
                    return Err(format!(
                        "timeout_msは1..={MAX_OPERATION_MS}で指定してください"
                    ));
                }
                Ok(())
            }
            Operation::WatchEdgesPolled {
                pre_pulse_low_ms,
                budget_ms,
                idle_timeout_us,
                glitch_filter_us,
                ..
            } => {
                validate_pre_pulse(*pre_pulse_low_ms)?;
                if *budget_ms == 0 || *budget_ms > MAX_OPERATION_MS {
                    return Err(format!(
                        "budget_msは1..={MAX_OPERATION_MS}で指定してください"
                    ));
                }
                if *idle_timeout_us == 0 || *idle_timeout_us > MAX_POLL_IDLE_TIMEOUT_US {
                    return Err(format!(
                        "idle_timeout_usは1..={MAX_POLL_IDLE_TIMEOUT_US}で指定してください"
                    ));
                }
                if *glitch_filter_us > *idle_timeout_us {
                    return Err(
                        "glitch_filter_usはidle_timeout_us以下で指定してください".to_string()
                    );
                }
                Ok(())
            }
            Operation::Read { .. } | Operation::Write { .. } | Operation::Release => Ok(()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Response {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// GPIO読み取りの結果（High=true）等、単一値を伴う成功レスポンス用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<bool>,
    /// I2C読み取りの結果等、バイト列を伴う成功レスポンス用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    /// `WatchEdges`の結果。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<EdgeEventWire>>,
}

#[derive(Debug, Serialize)]
pub struct EdgeEventWire {
    pub timestamp_ns: u64,
    pub rising: bool,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: None,
            edges: None,
        }
    }

    pub fn value(value: bool) -> Self {
        Self {
            ok: true,
            error: None,
            value: Some(value),
            bytes: None,
            edges: None,
        }
    }

    pub fn bytes(data: Vec<u8>) -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: Some(data),
            edges: None,
        }
    }

    pub fn edges(events: Vec<EdgeEventWire>) -> Self {
        Self {
            ok: true,
            error: None,
            value: None,
            bytes: None,
            edges: Some(events),
        }
    }

    pub fn locked_by(holder: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("locked_by:{holder}")),
            value: None,
            bytes: None,
            edges: None,
        }
    }

    pub fn malformed(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("malformed_request:{msg}")),
            value: None,
            bytes: None,
            edges: None,
        }
    }

    pub fn hw_error(msg: &str) -> Self {
        Self {
            ok: false,
            error: Some(format!("hw_error:{msg}")),
            value: None,
            bytes: None,
            edges: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(json: &str) -> Request {
        serde_json::from_str(json).expect("request fixture must parse")
    }

    #[test]
    fn polled_watch_uses_backward_compatible_idle_timeout() {
        let parsed = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges_polled":{
                    "pre_pulse_low_ms":null,
                    "budget_ms":15,
                    "pull":"up"
                }}
            }"#,
        );
        match parsed.op {
            Operation::WatchEdgesPolled {
                idle_timeout_us,
                glitch_filter_us,
                ..
            } => {
                assert_eq!(idle_timeout_us, DEFAULT_POLL_IDLE_TIMEOUT_US);
                assert_eq!(glitch_filter_us, 0);
            }
            _ => panic!("wrong operation"),
        }
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn client_can_override_polled_idle_timeout() {
        let parsed = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges_polled":{
                    "pre_pulse_low_ms":null,
                    "budget_ms":15,
                    "idle_timeout_us":2500,
                    "glitch_filter_us":10
                }}
            }"#,
        );
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn incompatible_operation_is_rejected_before_dispatch() {
        let parsed = request(
            r#"{
                "bus":{"type":"i2c","bus":1,"addr":118},
                "op":{"write":{"value":true}}
            }"#,
        );
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn unbounded_requests_are_rejected() {
        let huge_read = request(
            r#"{
                "bus":{"type":"uart","port":0,"baud_rate":9600},
                "op":{"read_bytes":{"length":1048577}}
            }"#,
        );
        assert!(huge_read.validate().is_err());

        let huge_wait = request(
            r#"{
                "bus":{"type":"gpio","pin":17},
                "op":{"watch_edges":{
                    "pre_pulse_low_ms":null,
                    "max_events":10,
                    "timeout_ms":60001
                }}
            }"#,
        );
        assert!(huge_wait.validate().is_err());
    }
}
