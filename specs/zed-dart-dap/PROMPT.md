# Objective
Implement `zed-dart-dap`: a Rust-native Zed debugger extension for Dart CLI apps, Flutter apps, and tests, reusing official SDK debug adapters (no custom DAP server).

# References
- Primary spec directory: `specs/zed-dart-dap/`
- Requirements: `specs/zed-dart-dap/requirements.md`
- Design: `specs/zed-dart-dap/design.md`
- Plan: `specs/zed-dart-dap/plan.md`

# Key Requirements
- Runtime must be Rust-only (no Node/TypeScript bridge).
- Reuse official adapters:
  - `dart debug_adapter` / `dart debug_adapter --test`
  - `flutter debug-adapter` / `flutter debug-adapter --test`
- Support core debug workflows: launch, attach, breakpoints, step in/out/over, variables, watch, call stack, debug console.
- Use Zed config model: per-worktree `.zed/debug.json` and global `debug.json` presets.
- No VS Code multi-root workspace semantics.
- Default templates should work for most Dart/Flutter/test projects without manual edits.
- DevTools must be user-invoked only (never auto-opened).
- v1 platform policy:
  - macOS officially tested and release-gated
  - Linux/Windows experimental (non-blocking)
- Toolchain policy: latest stable + previous 3 stable Dart/Flutter releases.
- No FVM support in v1.
- Release quality: zero known blocker/critical bugs and automated regression coverage for core flows.

# Implementation Guidance
- Follow `plan.md` step-by-step with TDD-first increments.
- Implement `get_dap_binary`, `dap_request_kind`, and `dap_config_to_scenario`.
- Build deterministic toolchain resolution (`which` + shell env + optional overrides).
- Produce strong diagnostics for tool resolution, config validation, adapter launch/exit.
- If Zed host APIs limit direct extension command actions for DevTools, implement a deterministic manual fallback UX that still satisfies “no auto-open”.

# Acceptance Criteria (Given-When-Then)
1. Given a Dart CLI project, when a default Dart launch template is started, then debugging works with breakpoints, stepping, variables, watch, call stack, and debug console.
2. Given a running Dart process with VM service, when attach is started, then session attaches and core debug features work.
3. Given a Flutter app project, when default Flutter launch is started on macOS, then debug session starts and core features work.
4. Given Dart/Flutter test files, when default test templates are started, then tests run under debug with breakpoint/step support.
5. Given a typical Dart/Flutter project, when shipped default templates are used unchanged, then at least one relevant debug flow starts successfully.
6. Given an active debug session, when user does not trigger DevTools, then DevTools is not auto-opened.
7. Given an active debug session, when user explicitly triggers DevTools, then DevTools endpoint/action path executes as designed.
8. Given supported toolchains on macOS, when CI runs, then core launch/attach/test flows pass across the defined version window.
9. Given Linux/Windows usage, when debug flows run, then behavior is available as experimental and non-blocking to release.
10. Given a release candidate, when triaged, then there are zero known blocker/critical bugs.

# Execution Notes
- Do not skip tests at each increment.
- Keep docs and schemas aligned with implemented behavior.
- Update checklist progress in `plan.md` as steps complete.
