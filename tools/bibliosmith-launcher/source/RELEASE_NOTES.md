## ZH

- 设置里新增「扫描 OCR」栏目：PaddleOCR（百度飞桨）和 MinerU 的 API Token 终于有界面可以填了。此前这两个 token 只能手写进仓库根的 .env；现在可以在设置里粘贴、测试连接（探测不消耗配额）、保存进系统钥匙串。.env 里已有的密钥继续有效，作为兜底——钥匙串里的密钥优先。
- 新建任务向导里的「PaddleOCR 凭据 / MinerU 凭据」芯片不再是写死的默认值（此前 Paddle 恒 ✗、MinerU 恒 ✓，跟实际配置无关，.env 里明明有 token 也显示红叉）。现在启动时按真实配置（钥匙串或 .env）探测，芯片仍可点击作为预检的手动覆盖。
- 这两项凭据只影响**扫描版 PDF**（没有文字层的书）；自带文字层的书走直读路线，跟 OCR 无关。

## EN

- Settings gains a "Scanned-book OCR" section: the PaddleOCR (Baidu) and MinerU API tokens finally have a UI. Until now they could only be hand-written into the repository-root .env; you can now paste a token in Settings, test the connection (the probe spends no quota), and store it in the system Keychain. A key already in .env keeps working as the fallback — a Keychain key takes precedence.
- The "PaddleOCR credentials / MinerU credentials" chips in the new-job wizard are no longer hard-coded defaults (previously Paddle was always ✗ and MinerU always ✓ regardless of reality — a working token in .env still showed a red cross). They now seed from the actual configured status (Keychain or .env) on launch, and stay clickable as manual preview overrides.
- These credentials only matter for **scanned PDFs** (books without a text layer); born-digital books take the direct-text route and never touch OCR.
