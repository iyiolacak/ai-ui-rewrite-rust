# Security policy

The decoders (`ai-ui-stream`, `ai-ui-data-stream`, `ai-ui-sdk-stream`) and `ai-ui-markdown` parse untrusted
network and model output; they are bounded and perform no I/O, but a bug there is a security issue.

Report vulnerabilities privately through GitHub: on the repository's **Security** tab choose **Report a
vulnerability** (<https://github.com/iyiolacak/ai-ui-rewrite-rust/security/advisories/new>). Please do not open a
public issue. Include a minimal input that reproduces the problem. Only the latest version is supported before 1.0.
