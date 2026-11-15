# Security policy

The decoders (`ai-ui-stream`, `ai-ui-data-stream`) and `ai-ui-markdown` parse untrusted
network and model output; they are bounded and perform no I/O, but a bug there is a security issue.

Report vulnerabilities privately to the maintainers (contact to be set when the repository is published) rather
than in a public issue. Include a minimal input that reproduces the problem. Only the latest minor version is
supported before 1.0.
