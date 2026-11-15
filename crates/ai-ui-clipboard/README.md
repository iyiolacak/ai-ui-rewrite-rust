# ai-ui-clipboard

Host-owned, result-aware clipboard text operations: native (arboard) and browser (`navigator.clipboard`), with no Slint or conversation-state dependency.

Call `Clipboard::copy` in the actual user gesture on the web. A queued request is not a successful copy: the completion reports the real outcome.

Part of [assistant-ui for Rust](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/README.md), a port of [assistant-ui](https://github.com/assistant-ui/assistant-ui) (MIT). Not affiliated with the assistant-ui project. Status and gaps: [docs/STATUS.md](https://github.com/iyiolacak/ai-ui-rewrite-rust/blob/main/docs/STATUS.md).
