# BiblioSmith Launcher 1.16.4

## ZH

BiblioSmith Launcher 1.16.4 修复 OCR 设置页在服务切换期间丢失进行中操作状态的问题，并随版本源码纳入 PR #202 中完成的代理协作约定与提示词方案研究材料。

### OCR 设置状态

- 在 PaddleOCR 与 MinerU 配置之间切换时，两个服务面板会保留各自的组件状态，不再因暂时离开当前面板而卸载。
- 保存、删除凭据或检测服务期间切换到另一服务，再切回时仍会显示正确的忙碌状态和结果消息。
- 进行中的操作保持禁用，避免用户因状态被重置而重复提交同一请求。
- 非当前服务面板继续从界面和辅助技术中隐藏，不改变一次只显示一个 OCR 配置的交互。

本版还包含 PR #202 合并的项目协作文档与提示词方案研究边界。它们不改变 Launcher 的运行时行为，但与本版本源码一同保留。当前仍仅提供 macOS Apple Silicon DMG；安装包使用 Developer ID 签名、Apple 公证与 stapling，并通过 Gatekeeper 验证。

## EN

BiblioSmith Launcher 1.16.4 fixes lost in-flight action state when switching OCR services and includes the agent collaboration guidance and prompt-pack research materials merged in PR #202 in the corresponding source snapshot.

### OCR settings state

- Switching between PaddleOCR and MinerU now preserves each service panel's component state instead of unmounting the temporarily inactive panel.
- If you switch services while saving, deleting credentials, or testing a service, returning to the original service still shows the correct busy state and result message.
- In-flight controls remain disabled, preventing duplicate submissions caused by an apparently reset interface.
- The inactive service remains hidden from both the visible interface and assistive technology, preserving the one-configuration-at-a-time interaction.

This release also carries the project collaboration documentation and prompt-pack research boundaries merged in PR #202. They do not change Launcher runtime behavior but are preserved with this version's source. The release remains a macOS Apple Silicon DMG, Developer ID signed, Apple notarized and stapled, and Gatekeeper verified.

## JA

BiblioSmith Launcher 1.16.4 は、OCR サービス切替時に進行中の操作状態が失われる問題を修正し、PR #202 で統合されたエージェント協働ガイドとプロンプトパック研究資料を対応するソーススナップショットに含めます。

### OCR 設定の状態保持

- PaddleOCR と MinerU を切り替えても、非アクティブ側をアンマウントせず、各サービスパネルのコンポーネント状態を保持します。
- 認証情報の保存・削除やサービス検査の途中で別サービスへ移動して戻った場合も、処理中状態と結果メッセージを正しく表示します。
- 実行中の操作は無効のまま維持され、画面がリセットされたように見えることで同じ要求を重複送信する事態を防ぎます。
- 非選択サービスは画面と支援技術の双方から引き続き隠され、一度に一つの OCR 設定だけを表示する操作を維持します。

本リリースには、PR #202 で統合されたプロジェクト協働文書とプロンプトパック研究の境界資料も含まれます。Launcher の実行時動作は変更しませんが、本バージョンのソースとともに保持されます。引き続き macOS Apple Silicon 向け DMG のみを提供し、Developer ID 署名、Apple 公証、stapling、Gatekeeper 検証を行います。
