# BiblioSmith Launcher 1.16.3

## ZH

BiblioSmith Launcher 1.16.3 强化 PDF 转换、Zotero 上传与 Book Pipeline 之间的证据交接，使中断后的继续执行建立在可核验的当前产物上。

### OCR 证据协调

- PDF 文本、PaddleOCR 与 MinerU 现在使用同一套版本化、哈希绑定的转换证据，记录来源 PDF、页范围、路由和每项产物的精确摘要。
- 每次转换写入独立且不可变的 run；显式换路由或重建旧完成记录时，不会覆盖仍然有效的旧产物。只有完整证据成功提交后，当前引用才会切换。
- Zotero 上传采用租约与幂等恢复。传输中断后可安全重试，避免重复创建 Markdown 附件；过期执行者不能覆盖新的提交。
- Book Pipeline 在启动、重试和恢复时都会重新核对当前 evidence、文件摘要、页覆盖与 Zotero 附件身份。缺失、漂移或不匹配时会明确阻断并给出安全错误码，不再根据目录或修改时间猜测完成状态。
- 用于协调的 conversion evidence 与 worker handoff 不保存书籍正文；产物引用限制在受控根目录内，Book Pipeline 的 mismatch 标记只暴露安全错误码。

本版仍仅提供 macOS Apple Silicon DMG。安装包使用 Developer ID 签名、Apple 公证与 stapling，并通过 Gatekeeper 验证；请从本 Release 手动下载升级。

## EN

BiblioSmith Launcher 1.16.3 strengthens the evidence handoff between PDF conversion, Zotero delivery, and Book Pipeline so interrupted work resumes only from verifiable current artifacts.

### OCR evidence reconciliation

- PDF text, PaddleOCR, and MinerU now share one versioned, hash-bound conversion-evidence contract covering the source PDF, selected pages, route, and exact digest of every artifact.
- Each conversion writes to an isolated immutable run. Explicit route changes and legacy completion regeneration no longer overwrite a valid prior bundle; the current reference changes only after the complete evidence commit succeeds.
- Zotero delivery now uses leases and idempotent recovery. Interrupted uploads can retry without creating duplicate Markdown attachments, and expired workers cannot overwrite a newer commit.
- Book Pipeline revalidates current evidence, file digests, page coverage, and Zotero attachment identity on start, retry, and resume. Missing, drifted, or mismatched evidence blocks with a safe error code instead of inferring completion from directories or modification times.
- Reconciliation conversion evidence and worker handoffs do not retain book content. Artifact references stay within a controlled root, and Book Pipeline mismatch markers expose only safe error codes.

This release remains a macOS Apple Silicon DMG. It is Developer ID signed, Apple notarized and stapled, and Gatekeeper verified. Download the DMG from this Release to upgrade manually.

## JA

BiblioSmith Launcher 1.16.3 は、PDF 変換、Zotero 配信、Book Pipeline 間の証拠引き継ぎを強化し、中断後の処理を検証可能な最新成果物からのみ再開します。

### OCR 証拠の整合

- PDF テキスト、PaddleOCR、MinerU は、元 PDF、選択ページ、ルート、各成果物の正確なダイジェストを含む、共通のバージョン付きハッシュ拘束型変換証拠を使用します。
- 各変換は独立した不変 run に書き込みます。明示的なルート変更や旧完了レコードの再生成でも、有効な旧成果物を上書きせず、完全な evidence commit が成功した後にだけ現在参照を切り替えます。
- Zotero 配信はリースと冪等な復旧に対応します。中断したアップロードを Markdown 添付の重複なしで再試行でき、期限切れ worker は新しい commit を上書きできません。
- Book Pipeline は開始、再試行、再開のたびに、現在の evidence、ファイルダイジェスト、ページ範囲、Zotero 添付の同一性を再検証します。欠落、変化、不一致がある場合は安全なエラーコードで停止し、ディレクトリや更新時刻から完了を推測しません。
- 整合確認用の conversion evidence と worker handoff は書籍本文を保持しません。成果物参照は管理対象のルート内に制限され、Book Pipeline の mismatch marker は安全なエラーコードだけを公開します。

本リリースも macOS Apple Silicon 向け DMG のみです。Developer ID 署名、Apple 公証、stapling、Gatekeeper 検証を行います。この Release から DMG を手動でダウンロードして更新してください。
