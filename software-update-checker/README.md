# software-update-checker（実験版）

pom.xml や SBOM から依存ソフトウェアを一覧表示し、OSS について次の 3 点を確認するデスクトップ GUI ツールです。

- **最新バージョン**: 同じ系列（メジャー.マイナー）内の最新版、全体の最新版、系列ごとの最新版の一覧、更新の大きさ（パッチ／マイナー／メジャー）、非推奨かどうか、最終リリース日
- **必要な Java**（Maven）: 系列ごとの最新版の jar から、実行に必要な Java を判定（logback 1.3 系は Java 8、1.4 系以降は Java 11 のように、系列で要件が変わるものを見分けるため）
- **脆弱性**: 既知の脆弱性（GHSA / CVE）、深刻度、CVSS スコア、修正済みバージョン
- **サポート終了（EOL）**: 製品のサイクルごとのサポート状況・終了日・要件（Tomcat の必要 Java など）

一覧表示のほか、依存関係のツリー表示（絞り込み時は該当する部品へ至る経路だけを表示）ができます。

同じリポジトリにある Java 製 CLI（`dependency-auto-update-tool`）とは独立したサブプロジェクトで、コードは共有していません。

## 対応入力

| 形式 | 推移的依存 | 備考 |
|---|---|---|
| pom.xml | なし | このファイル内の `${property}`・dependencyManagement・parent の version だけを解決。parent と import スコープの BOM も一覧に出す |
| pom.xml →「Maven で SBOM 生成」 | あり | ローカルの `mvn` で `cyclonedx-maven-plugin` を実行し、生成された SBOM を取り込む。対象プロジェクトの pom.xml は変更しない（出力先は `target/software-update-checker-bom.json`） |
| CycloneDX JSON / XML | あり | `dependencies` があれば直接依存／推移的依存を判定 |
| SPDX 2.x JSON | あり | `relationships` の DEPENDS_ON / DEPENDENCY_OF などから判定 |

ファイルはウィンドウへのドラッグ＆ドロップか、「ファイルを開く」で読み込みます。

## データソース（通信先）

このツールが通信するのは、[`src-tauri/src/sources.rs`](src-tauri/src/sources.rs) に登録した次の接続先だけです。すべて API キー不要です。

| データソース | 接続先 | 用途 | 送信される情報 | キャッシュ期間 |
|---|---|---|---|---|
| [deps.dev](https://docs.deps.dev/api/v3/) | api.deps.dev | 最新版・系列ごとの最新版・非推奨・公開日・ライセンス | パッケージ名とバージョン（URL） | 24 時間 |
| [OSV.dev](https://google.github.io/osv.dev/api/) | api.osv.dev | 脆弱性 | purl とバージョン（POST 本文）、脆弱性 ID | 6 時間（ID 一覧）／24 時間（詳細） |
| [endoflife.date](https://endoflife.date/docs/api/v1/) | endoflife.date | サポート終了・サイクルごとの要件 | なし（製品一覧をまとめて取得し、照合は手元） | 7 日 |
| Maven Central | repo1.maven.org | jar から必要な Java を判定（詳細画面を開いたとき、ナレッジに記録が無いものだけ） | groupId・artifactId・バージョン（URL）。jar は末尾と数クラス分だけ Range で部分取得 | ナレッジに記録 |
| ローカルの Maven | （mvn が設定に従って通信） | 「Maven で SBOM 生成」 | Maven の依存解決による通信。このツールの外で行われるため、ログには実行の開始・終了のみ | — |

- 設定画面でデータソースごとに ON/OFF できます。OFF にしたデータソースへの通信は、HTTP 層で**送信前に遮断**し、遮断したこと自体も記録します。登録されていないホストへの通信も同様に遮断します。
- キャッシュはアプリのキャッシュディレクトリ（Windows は `%LOCALAPPDATA%\org.tamacat.software-update-checker\http-cache.json`）に保存します。
- HTTP(S) プロキシは環境変数 `HTTPS_PROXY` / `HTTP_PROXY` に従います。

### 通信の制限（相手のサービスに負荷をかけないために）

すべての送信は [`src-tauri/src/netguard.rs`](src-tauri/src/netguard.rs) の流量制御を通ります。設定画面の「通信」で変更できますが、下表の範囲を超える値は自動で範囲内に丸めます。

| 項目 | 既定 | 範囲 |
|---|---|---|
| 失敗時の再試行回数（5xx・429・通信失敗のときだけ。404 などは再試行しない） | 2 回 | 0〜5 |
| 再試行の待ち時間（回ごとに倍、少しずらす） | 1000 ms | 200〜30000 |
| 待ち時間の上限（`Retry-After` がこれを超えたら再試行しない） | 30000 ms | 1000〜120000 |
| 1 要求のタイムアウト | 60 秒 | 5〜300 |
| 同時に送る要求の数 | 6 | 1〜16 |
| 同じ接続先への最小間隔 | 100 ms | 0〜5000 |
| 同じ接続先への 1 分あたりの上限（超えそうなら待つ） | 300 件 | 10〜1200 |
| 同じ要求（URL・Range・本文が同じ）を 1 分間に送ってよい回数（超えたら遮断。不具合による暴走を止める） | 3 回 | 1〜20 |
| 連続失敗で送信を一時停止するまでの回数 | 5 回 | 1〜50 |
| 一時停止する時間 | 120 秒 | 10〜3600 |

`Retry-After` が付いた 429 / 503 はその指示に従います。待機・遮断・一時停止はすべて通信ログに残ります。

### 通信ログ

外部への通信 1 件ごと（実際の通信・キャッシュ利用・遮断・失敗）と、取り込み・チェック・Maven 実行の開始と終了を記録します。

- 画面の「通信ログ」タブで、データソースごとの件数・受信量と、記録の一覧をリアルタイムに確認できます。
- ファイルはアプリのログディレクトリ（Windows は `%LOCALAPPDATA%\org.tamacat.software-update-checker\logs\`）に、日別の JSON Lines（`activity-YYYY-MM-DD.jsonl`、日付は UTC）で追記します。30 日より古いファイルは起動時に削除します。

EOL の製品特定は、①endoflife.date の purl 識別子との完全一致、②同梱の対応表 [`src-tauri/resources/eol-mapping.json`](src-tauri/resources/eol-mapping.json)、③同じ namespace（groupId）を持つ製品が 1 つだけならその製品、の順に行います。②③で特定したものは画面に「推定」と表示します。

## Java 要件のナレッジ

「どのライブラリのどのバージョンにどの Java が必要か」を JSON ファイルに記録し、使い回します（[`src-tauri/src/knowledge.rs`](src-tauri/src/knowledge.rs)）。

- **自動記録**: 詳細画面で Maven の部品を開くと、系列ごとの最新版の jar から必要な Java を判定し、手元のファイル（Windows は `%APPDATA%\org.tamacat.software-update-checker\java-requirements.json`）に `source: "jar"` で記録します。次からは通信せずに記録から答えます。
- **手動記録**: 詳細画面の ✎、または「Java 要件」タブから登録します。バージョン単位のほか、系列（`series: "1.4"` → 1.4.x すべて）や groupId 全体（`artifact: "*"`）にも当てはめられ、理由や出典をメモに残せます。手動記録は自動判定や取り込みで上書きされません。
- **共有**: 「書き出し…」で書き出したファイルを、チームの共有フォルダや git で共有できます。他の人は「取り込み…」で手元に取り込むか、「共有ファイル」に追加して読み込み専用で参照します。
- **優先順位**: より具体的な記録（artifact 一致 > `*`、バージョン一致 > 系列 > 指定なし）→ 手動 > jar 判定 → 手元 > 共有ファイル。
- Maven Central を無効にしていても、ナレッジに記録があるものは表示できます。
- 記録日時（`checkedAt`）は UTC の ISO 8601 日時で保存し、画面では設定したタイムゾーンの日付で表示します。手で書く場合は `YYYY-MM-DD` でも構いません。

見本として、今回 jar から確認した logback・Tomcat・JAXB の系列ごとの必要 Java を [`examples/java-requirements.example.json`](examples/java-requirements.example.json) に入れています。

```json
{
  "format": "software-update-checker/java-requirements",
  "formatVersion": 1,
  "entries": [
    { "ecosystem": "maven", "group": "ch.qos.logback", "artifact": "*", "series": "1.4", "java": "11",
      "source": "manual", "checkedAt": "2026-10-03", "note": "1.4 系から Java 11 必須" },
    { "ecosystem": "maven", "group": "ch.qos.logback", "artifact": "logback-core", "version": "1.3.16", "java": "8",
      "classMajor": 52, "source": "jar", "checkedAt": "2026-10-02" }
  ]
}
```

## 表示設定

設定画面の「表示」で、この端末での表示を変えられます（チェックの設定とは別に保存します）。

- **表示言語**: 日本語 / English。画面の文言に加えて、バックエンドが作る警告・エラー・通信ログのメッセージも切り替わります（切り替え前に記録されたログはそのままです）。
- **文字サイズ**: 85%〜150%。
- **タイムゾーン**: OS の設定に従うか、IANA のタイムゾーン名（`Asia/Tokyo`、`UTC` など）を指定します。通信ログの時刻や公開日・記録日の表示に使います。ログファイルとナレッジファイルには常に UTC で記録します。

言語を足すときは、画面は [`src/lib/i18n.ts`](src/lib/i18n.ts) に辞書を、バックエンドは [`src-tauri/src/i18n.rs`](src-tauri/src/i18n.rs) と `tr!` の呼び出しに訳を追加します。

## 構成

```
software-update-checker/
├─ src/                    フロントエンド（Svelte 5 + TypeScript、表示のみ）
│  ├─ App.svelte           画面全体・状態管理
│  └─ lib/                 一覧・詳細・集計カード・設定ダイアログ、型定義、CSV/JSON 出力
└─ src-tauri/              バックエンド（Rust、ロジックはすべてこちら）
   ├─ src/model.rs         共通データモデル（フロントの lib/types.ts と対応）
   ├─ src/importers/       入力形式ごとの取り込み（Importer トレイト）
   ├─ src/checks/          チェック種別ごとの処理（versions / vulns / eol / java、CVSS 計算）
   ├─ src/sources.rs       データソース（通信先）の登録表
   ├─ src/http.rs          HTTP クライアント・通信先の許可制御・再試行・ファイルキャッシュ
   ├─ src/netguard.rs      流量制御（間隔・上限・繰り返し遮断・一時停止）
   ├─ src/knowledge.rs     Java 要件のナレッジファイル
   ├─ src/activity.rs      通信・操作ログ
   ├─ src/i18n.rs          表示言語の切り替え（tr! マクロ）
   └─ src/commands.rs      フロントから呼べるコマンド
```

### セキュリティ上の設計

- **外部 API はすべて Rust 側から呼ぶ**: 画面（WebView）には外部への通信を許可していません。
- **CSP**: `script-src 'self'; style-src 'self'` などで、インラインのスクリプトやスタイルを禁止しています。画面のスタイルは CSS ファイルと、JavaScript から個別に設定するもの（Svelte の `style:` 指令）だけを使います。
- **ファイルの書き込み先は利用者が選んだ場所だけ**: CSV / JSON 出力とナレッジの書き出しでは、保存先のダイアログをバックエンド側で出し、そこで選ばれたパスにだけ書き込みます。画面からパスを指定して書き込むコマンドはありません。
- **開ける外部ページを限定**: ブラウザで開けるのは `https://osv.dev/*` と `https://endoflife.date/*` だけです（`capabilities/default.json`）。
- **信頼できない入力への備え**:
  - XML は DTD を受け付けません（XXE・Billion Laughs 対策）。
  - jar のクラスファイルは先頭 64 バイトまでしか展開しません（展開爆弾対策）。
  - HTTP 応答は 50MB で打ち切ります。

CSP 違反がないかは、模擬バックエンド入りのビルド（`npm run build:mock`、出力は `dist-mock/`）を同じ CSP 付きで配信して確認できます。本番ビルド（`npm run build`）には模擬バックエンドは含まれません。

### 対応を広げるとき

- **入力形式を足す**（package-lock.json、requirements.txt、Gradle など）: `src-tauri/src/importers/` に `Importer` トレイトの実装を追加し、`importers::registry()` に登録します。
- **エコシステムを足す**: `model::Ecosystem` に値を追加し、purl の type と deps.dev の system 名との対応を書きます。最新版・脆弱性チェックは purl ベースなので、多くの場合それだけで動きます。
- **チェックを足す**（ライセンス判定、社内リポジトリ照会など）: `src-tauri/src/checks/` にモジュールを追加し、`checks::run_checks` から呼びます。結果は `model::CheckResult` に項目を追加します。新しい通信先を使う場合は `sources.rs` への登録が必須です（登録しないと遮断されます）。

## 開発

### 前提

- Node.js 20 以上
- Rust（stable）
- Windows: Microsoft C++ Build Tools と WebView2（Windows 11 は標準搭載）
- macOS / Linux: [Tauri の前提パッケージ](https://v2.tauri.app/start/prerequisites/)

### コマンド

```bash
npm install
```

```bash
npm run tauri dev
```

```bash
npm run tauri build
```

`npm run tauri build` で、`src-tauri/target/release/bundle/` 配下にインストーラー（Windows は MSI / NSIS）が作られます。

インストーラーを作らず、実行ファイル 1 つだけを作る場合:

```bash
npm run tauri build -- --no-bundle
```

`src-tauri/target/release/software-update-checker.exe`（約 5MB）ができます。

- **単体で動く**: 画面と同梱データは exe に埋め込まれており、Visual C++ ランタイムなどの DLL も不要です。コピーするだけで使えます。
- **必要な環境**: Microsoft Edge WebView2 ランタイム（Windows 11 と、更新済みの Windows 10 には標準搭載）。
- **任意**: 「Maven で SBOM 生成」を使う場合だけ、PATH に Maven が必要です。
- **設定・ナレッジ・ログ・キャッシュ**: exe の横ではなく、ユーザーのアプリデータフォルダ（`%APPDATA%` / `%LOCALAPPDATA%` の `org.tamacat.software-update-checker`）に保存します。
- **コード署名**: していないため、初回起動時に Windows SmartScreen の警告が出ることがあります。

テストと型チェック:

```bash
cd src-tauri && cargo test
```

```bash
npm run check
```

### 画面だけを確認する

`npm run dev` で起動した開発サーバー（http://localhost:1420）を通常のブラウザで開くと、Tauri の外であることを検知して模擬バックエンド（`src/lib/mock.ts`）で動きます。外部 API や Rust 側のビルドなしで画面を確認できます。本番ビルドには含まれません。

模擬データ `src/lib/fixtures/sample.json` は、実際のチェック結果を保存したものです。次のコマンドで作り直せます（`SUC_LIVE_INPUT` に取り込みたい pom.xml や SBOM のパスを指定）。

```bash
cd src-tauri && SUC_LIVE_INPUT=path/to/pom.xml SUC_LIVE_DUMP=../src/lib/fixtures/sample.json cargo test live -- --ignored
```

### アイコン

`app-icon.png`（1024×1024）から `npx tauri icon app-icon.png` で `src-tauri/icons/` を作り直せます。

## 未対応・今後の課題

- **自動更新**: 未設定。`tauri-plugin-updater` を追加し、`tauri signer generate` で作った署名鍵の公開鍵と、更新情報の配信 URL（GitHub Releases など）を `tauri.conf.json` に設定する必要があります。秘密鍵はリポジトリに入れず CI のシークレットで管理します。
- **配布用の署名**: Windows のコード署名、macOS の公証（Apple Developer Program）。
- HTML レポート、前回結果との差分、複数ファイルの横断表示。
- Java 要件の判定は Maven（jar）のみ。npm の `engines` や PyPI の `requires_python` などは未対応。
- pom.xml 直接読込での親 POM / BOM の解決（現状は「Maven で SBOM 生成」で代替）。
- OSV の `querybatch` は 1 パッケージあたりの結果が多いとページングされますが、2 ページ目以降は未取得です。
- CVSS v4.0 のスコア計算（深刻度ラベルは GHSA の値を使うので表示はされます）。
