//! UARTハードウェアアクセス（FEATURE_PRIORITY.md Tier 1）。
//!
//! カーネルのtermios API（`tcgetattr`/`tcsetattr`）でraw modeの8N1に設定した
//! 上で、通常のファイル読み書きとして扱う。GPIO/I2C/SPIと同じく、既存の
//! カーネルドライバ（シリアルドライバ）に実際の信号生成・受信を委ねる。
//!
//! 標準的なraw 8N1 deviceを対象とし、対応baud rateは明示的に限定する。

use crate::error::HwError;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;

pub struct UartPort {
    file: File,
}

impl UartPort {
    pub fn open(device: &str, baud_rate: u32) -> Result<Self, HwError> {
        let speed = Self::baud_to_speed(baud_rate)?;

        // O_NOCTTYで、このシリアルポートがプロセスの制御端末になるのを防ぐ
        // （長時間稼働するデーモンが意図せずSIGHUP等を受け取らないようにする）。
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(device)
            .map_err(|e| HwError::OpenFailed(format!("{device}: {e}")))?;

        // SAFETY: `termios`はこの関数内でのみ使うスタック上の値。
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: fdはこの関数内で有効、`termios`は書き込み先として妥当なサイズ。
        if unsafe { libc::tcgetattr(file.as_raw_fd(), &mut termios) } < 0 {
            return Err(HwError::OpenFailed(format!(
                "tcgetattr {device}: {}",
                std::io::Error::last_os_error()
            )));
        }

        // SAFETY: `termios`はtcgetattrで初期化済みの有効な値。
        unsafe {
            libc::cfsetispeed(&mut termios, speed);
            libc::cfsetospeed(&mut termios, speed);
        }

        // raw mode・8N1・フロー制御無し。MH-Z19C等の単純なバイナリプロトコル
        // 用に、行編集・エコー・特殊文字変換を全て無効化する。
        termios.c_cflag &= !(libc::PARENB | libc::CSTOPB | libc::CSIZE);
        termios.c_cflag |= libc::CS8 | libc::CLOCAL | libc::CREAD;
        termios.c_iflag &=
            !(libc::IXON | libc::IXOFF | libc::IXANY | libc::ICRNL | libc::INLCR | libc::IGNBRK);
        termios.c_oflag &= !libc::OPOST;
        termios.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ISIG | libc::IEXTEN);
        // VMIN=0 and VTIME=10 (1.0 seconds): read() returns when data arrives
        // と同じ挙動に合わせる。VMIN=1・VTIME=0（純粋なブロッキング）だと、
        // センサー未接続でデータが一切来ない場合にread()が無期限にブロックし、
        // デーモンのワーカースレッドを止めてしまう（実機テストで発覚）。
        termios.c_cc[libc::VMIN] = 0;
        termios.c_cc[libc::VTIME] = 10;

        // SAFETY: fdはこの関数内で有効、`termios`は設定済みの有効な値。
        if unsafe { libc::tcsetattr(file.as_raw_fd(), libc::TCSANOW, &termios) } < 0 {
            return Err(HwError::OpenFailed(format!(
                "tcsetattr {device}: {}",
                std::io::Error::last_os_error()
            )));
        }

        Ok(Self { file })
    }

    fn baud_to_speed(baud: u32) -> Result<libc::speed_t, HwError> {
        Ok(match baud {
            1_200 => libc::B1200,
            2_400 => libc::B2400,
            4_800 => libc::B4800,
            9_600 => libc::B9600,
            19_200 => libc::B19200,
            38_400 => libc::B38400,
            57_600 => libc::B57600,
            115_200 => libc::B115200,
            _ => return Err(HwError::InvalidChannel(baud)),
        })
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, HwError> {
        self.file
            .read(buf)
            .map_err(|e| HwError::TransferFailed(format!("uart read: {e}")))
    }

    pub fn write(&mut self, data: &[u8]) -> Result<usize, HwError> {
        self.file
            .write(data)
            .map_err(|e| HwError::TransferFailed(format!("uart write: {e}")))
    }
}
