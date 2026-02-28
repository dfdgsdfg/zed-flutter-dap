Q1: What is the primary goal of "zed-dart-dap" in one sentence (for example: debug Dart CLI apps, Flutter apps, tests, or all of these)?
A1: All of these: Dart CLI apps, Flutter apps, and tests.
Q2: Who are the target users and platforms for v1 (for example: Zed extension users on macOS/Linux/Windows, and any platform priorities)?
A2: Target users are Zed extension users, with macOS as the priority platform for v1, while also supporting Linux and Windows.
Q3: For v1, which debugging workflows are mandatory on day one: launch, attach, breakpoints, step in/out/over, variable inspection, watch expressions, call stack, and debug console?
A3: Mandatory v1 workflows include launch, attach, breakpoints, step in/out/over, variable inspection, watch expressions, call stack, and debug console; additionally, Flutter debugging should support opening web-based debug tooling.
Q4: Should "Flutter debug tool open to web" mean launching Flutter DevTools automatically in the user’s browser from a Zed debug session?
A4: DevTools should not open automatically; users should invoke it manually from a Zed command or equivalent Zed UI action.
Q5: What should be explicitly out of scope for v1 (for example: remote device debugging, hot reload controls in UI, profiling timelines, multi-root workspaces)?
A5: No explicit functional scope exclusions for v1; target is full-featured debugging support.
Q6: Should “full feature” also include less common scenarios in v1, such as debugging tests with custom runners, workspace-level launch configurations, and attach-to-running process by PID?
A6: User raised concern that Zed may not support workspace-level configurations/workspaces in the VS Code sense.
Q7: Given current Zed behavior, should v1 use per-worktree `.zed/debug.json` plus optional global `debug.json` presets instead of workspace-level (multi-root) launch configurations?
A7: Yes; v1 should use per-worktree `.zed/debug.json` with optional global `debug.json` presets, and not target multi-root workspace launch configurations.
Q8: What are the top 3 success criteria for v1 at release time (for example: setup under N minutes, crash-free sessions > X%, supports Dart/Flutter/test templates out of the box)?
A8: Success criterion: default templates should work for most Dart/Flutter/test projects without manual edits.
Q9: What is your target for initial setup time (for example: first successful debug run in under 5 minutes for a typical project)?
A9: Initial setup time is not a priority; primary success is robust features that work well with minimal bugs.
Q10: For release quality, should v1 require zero known blocker/critical bugs and automated regression coverage for core debug flows (launch/attach/breakpoints/stepping/variables/call stack/debug console)?
A10: Yes; v1 should ship with zero known blocker/critical bugs and automated regression coverage for core debug flows.
Q11: Which runtime/toolchain versions should v1 officially target (minimum Dart SDK, Flutter SDK channel/version, and whether FVM-managed projects must be supported)?
A11: v1 will not support FVM-managed projects. Target modern Dart/Flutter versions with a rolling compatibility policy around current releases and recent prior versions.
Q12: To confirm version policy, should v1 support the latest stable Dart/Flutter plus the previous 3 stable releases for each, and test this on macOS/Linux/Windows?
A12: Confirmed version policy (latest stable plus previous 3). v1 should officially test only on macOS; Linux/Windows features may exist but are not part of the tested support matrix.
Q13: How should Linux/Windows status be communicated in v1: experimental support with no CI guarantees, or best-effort support with clear known limitations listed?
A13: Linux/Windows should be marked as experimental support in v1.
Q14: Should zed-dart-dap reuse the official Dart/Flutter debug adapters provided by the SDK toolchain (invoked by commands), rather than implementing a custom Dart DAP server from scratch?
A14: Reference and reuse official Dart/Flutter debug adapter behavior/tooling, while implementing the Zed integration as a native Rust extension.
Q15: For v1, should the extension runtime be Rust-only (no Node/TypeScript bridge), invoking `dart`/`flutter` toolchain commands directly?
A15: Yes; v1 should use a Rust-only runtime and invoke `dart`/`flutter` toolchain commands directly.
Q16: If required SDK tools are missing or misconfigured, should v1 fail fast with actionable diagnostics (detected paths, missing command, suggested fixes) inside Zed UI?
A16: If SDK tools are missing/misconfigured, v1 should log diagnostic details primarily for debugging purposes (not heavy user-facing remediation flows).
Q17: Are requirements clarification complete, or do you want to continue clarifying details (or switch to preliminary research)?
A17: Requirements clarification is complete.
