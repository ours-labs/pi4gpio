# Pi4gpio

[English](README.md) | 日本語

Pi4gpioは、Raspberry Pi 4向けのローカルなハードウェアアクセス用デーモンです。GPIO、I2C、SPI、UARTに共通のAPIを提供し、クライアントごとのリソース所有権を管理します。

## リリース済みの機能

バージョン0.1.2では、次の機能を提供します。

- GPIOの入力、出力、タイムスタンプ付きエッジ監視
- I2C、SPI、UARTのバイト単位の操作
- 接続ごとの所有権管理と、切断後の自動クリーンアップ
- 転送サイズ、待機時間、エッジ数の上限設定
- Unixドメインソケットによる通信
- 追加依存のないPythonクライアント

ハードウェアPWM、サーボパルス、波形生成、リモート制御、pigpiodプロトコル互換性は0.1.2では利用できません。正確な対応範囲は[docs/CAPABILITIES.md](docs/CAPABILITIES.md)に記載しています。

## 未リリースの機能

現在の`main`ブランチには、ネイティブプロトコルのバージョンと機能を調べる読み取り専用の`hello`操作が追加されています。既存のバージョン1のハードウェア要求形式は維持されています。この操作は0.1.2リリースには含まれず、pigpiodの通信プロトコル互換性を提供するものではありません。

## セキュリティ上の境界

プロトコルはローカル利用専用です。TCPプロキシやVPNブリッジを通してUnixソケットを外部へ公開しないでください。将来リモート通信を導入する場合は、認証、認可、リプレイ対策、レート制限、監査の設計を独立してレビューする必要があります。

Pi4gpioは参加するクライアント間を調停しますが、他のプロセスによるハードウェアデバイスへの直接アクセスを防ぐことはできません。専用サービスアカウントとOSのデバイス権限を使い、デーモンを迂回した直接アクセスを制限してください。

公開脅威モデルと配備要件は[docs/SECURITY_MODEL.md](docs/SECURITY_MODEL.md)を参照してください。

## 責任の分担

デーモンはバスの低水準操作、リソースの調停、切断時のクリーンアップを担当します。センサーデータのデコード、値の検証、永続化、サンプリングのスケジュールはクライアント側の責任です。

## リポジトリ構成

- `crates/pi4gpio-daemon`: Unixソケットサーバーとリソース調停
- `crates/pi4gpio-hw`: Linuxのハードウェアアクセス
- `clients/python`: `pi4gpio_client`パッケージ
- `systemd`: 汎用サービスの例
- `docs`: 公開用の機能・セキュリティ文書

## ビルド

64ビットOSを使用するRaspberry Pi 4向け:

```bash
cargo build --release --target aarch64-unknown-linux-gnu
```

## テスト

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
PYTHONPATH=clients/python python3 -m unittest discover -s clients/python/tests -v
python3 -m unittest discover -s protocol/tests -v
```

実機での受入検証結果には、Piの機種、OSバージョン、アーキテクチャ、対象インターフェース、検証時間、サンプル数、エラー数、ソフトウェアバージョンを記載してください。ホスト名、アドレス、アカウント名、ローカルパスは公開しないでください。

## ライセンス

[MIT](LICENSE)
