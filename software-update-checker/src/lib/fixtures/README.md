# 画面確認用の模擬データ

`sample.json` は、開発時に通常のブラウザで画面を確認するための模擬バックエンド（`src/lib/mock.ts`）が返すデータです。本番ビルドには含まれません。作り方はプロジェクトの README の「画面だけを確認する」を参照してください。

中身は、公開されている tamacat-httpd の CycloneDX SBOM を実際にチェックした結果で、次の外部サービスから取得したデータを含みます。

| データ | 取得元 | ライセンス |
|---|---|---|
| バージョン一覧・公開日・ライセンス | [deps.dev](https://deps.dev/)（Open Source Insights） | deps.dev が生成したデータは [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| 脆弱性の ID・概要・深刻度・修正版 | [OSV.dev](https://osv.dev/) 経由の [GitHub Advisory Database](https://github.com/github/advisory-database) | [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/) |
| サポート終了日・サイクルの要件 | [endoflife.date](https://endoflife.date/) | [MIT](https://github.com/endoflife-date/endoflife.date/blob/master/LICENSE) |
| 必要な Java のバージョン | Maven Central の jar から判定した結果 | 判定結果（事実情報） |

これらのデータは取得時点のもので、変更は加えていません（画面表示用に必要な項目だけを抜き出しています）。
