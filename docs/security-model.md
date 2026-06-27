# Security Notes

[中文](zh-CN/security-model.md) | English

SuperNova is a local desktop application. It can interact with local workspace data, provider configuration, runtime services, and generated artifacts. This document describes the public security posture for developers and users; it does not publish internal security-control details.

## Public Posture

| Area | Public rule |
| --- | --- |
| Provider configuration | Keep provider keys and local configuration out of Git, screenshots, logs, and issues. |
| Workspace data | Treat local files, paths, and generated artifacts as user data. Redact before sharing. |
| Runtime access | Treat local runtime access as sensitive. Public docs describe responsibilities, not implementation controls. |
| Tool execution | File changes, terminal work, and artifact generation should be tied to runtime evidence and user-visible state. |
| Security claims | Do not claim broad security completion without a current dedicated audit. |

## What Public Docs Can Say

Public documentation can explain:

- the local-first architecture;
- that provider configuration is sensitive local state;
- that tool execution is controlled by runtime contracts;
- that execution truth and UI projection are separate;
- that broad security claims require current audit evidence.

Public documentation should not include:

- secrets or local access material;
- private local paths, account identifiers, or unredacted screenshots;
- internal access-control exchanges;
- internal credential handling details;
- exploit narratives, bypass examples, or security test material;
- implementation-level defensive check order.

## Contributor Rules

Before opening a PR or publishing a screenshot:

1. Check that no provider keys or private local state are included.
2. Redact workspace paths, account names, and private file names.
3. Keep implementation-level security analysis in internal audit material.
4. Use [Validation](validation.md) for current-state claims rather than relying on old local evidence.

## Reporting Security Issues

If you believe you found a security issue, report the behavior and impact without publishing exploit instructions or sensitive local material in a public thread.
