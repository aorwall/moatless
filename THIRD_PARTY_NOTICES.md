# Third-party notices

Moatless is licensed under the [Apache License 2.0](LICENSE). The images it
publishes also contain third-party software under the licenses below. Each
component keeps its own license, and none is relicensed under Moatless's.

Claude Code is a trademark of Anthropic PBC, Codex of OpenAI, and T3 Code of
T3 Tools, Inc. Moatless is not affiliated with, endorsed by or sponsored by any
of them.

## Agent CLIs in the sandbox images

`aorwall/moatless-sandbox-base` and every image built on it install these from
their publishers' npm packages, unmodified. `docker/cli-versions.env` is the
source of the versions below; `.github/scripts/check-third-party-notices.sh`
fails CI when they disagree.

### Claude Code

<!-- renovate: datasource=npm depName=@anthropic-ai/claude-code -->
`@anthropic-ai/claude-code` 2.1.332

Proprietary. The package's `LICENSE.md` reads:

> © Anthropic PBC. All rights reserved. Use is subject to the Legal Agreements
> outlined here: https://code.claude.com/docs/en/legal-and-compliance.

Each user signs in with their own Anthropic API key, Claude plan or
cloud-provider credentials.

### OpenAI Codex

<!-- renovate: datasource=npm depName=@openai/codex -->
`@openai/codex` 0.172.0

Apache License 2.0, https://github.com/openai/codex. Its `NOTICE` reads:

```text
OpenAI Codex
Copyright 2025 OpenAI

This project includes code derived from [Ratatui](https://github.com/ratatui/ratatui), licensed under the MIT license.
Copyright (c) 2016-2022 Florian Dehau
Copyright (c) 2023-2025 The Ratatui Developers
```

### Pi coding agent

<!-- renovate: datasource=npm depName=@earendil-works/pi-coding-agent -->
`@earendil-works/pi-coding-agent` 1.6.1

MIT License, Copyright (c) 2026 Earendil Inc., https://github.com/earendil-works/pi.

### Other agents

| Component | License |
|---|---|
| Antigravity CLI (`agy`) | Proprietary, Google LLC |
| mini-swe-agent, LiteLLM, MCP Python SDK | MIT |

## Tools in the sandbox images

| Component | License |
|---|---|
| Node.js, Bun, pnpm, Vite+, Task, GitHub CLI, yq, git-delta | MIT |
| Playwright | Apache-2.0; its Chromium build ships its own credits |
| Docker CLI, AWS CLI v2, Session Manager plugin, kubectl, Helm, crane | Apache-2.0 |
| Rust toolchain | MIT or Apache-2.0 |
| Terraform | Business Source License 1.1 |
| Ubuntu 24.04 packages | Their own; see `/usr/share/doc/*/copyright` in the image |

## T3 Code, in `aorwall/moatless-t3`

The web UI is an unofficial fork of [T3 Code](https://github.com/pingdotgg/t3code),
built from `soaplabs/t3code`. The image serves its npm dependencies' licenses
at `/third-party-licenses.json`.

```text
MIT License

Copyright (c) 2026 T3 Tools Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Rust dependencies

The backend and `moat` binaries link the crates listed in `Cargo.lock`, each
under its own license: MIT or Apache-2.0 for nearly all, plus Unicode-3.0,
BSD-3-Clause, ISC, CDLA-Permissive-2.0 (`webpki-roots`), CC0-1.0 and MPL-2.0
(`option-ext`, source at https://github.com/soc/option-ext).
