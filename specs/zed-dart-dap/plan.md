# zed-dart-dap Implementation Plan

## Checklist
- [ ] Step 1: Create extension skeleton and debugger adapter registrations
- [ ] Step 2: Define adapter schemas and configuration validation boundaries
- [ ] Step 3: Implement request-kind resolution and normalized target classification
- [ ] Step 4: Implement toolchain discovery and deterministic launch descriptor builder
- [ ] Step 5: Deliver Dart launch and attach end-to-end sessions
- [ ] Step 6: Deliver Dart test debugging via official test adapter mode
- [ ] Step 7: Deliver Flutter launch and attach end-to-end sessions
- [ ] Step 8: Deliver Flutter test debugging via official test adapter mode
- [ ] Step 9: Add New Debug Session conversion and template defaults
- [ ] Step 10: Add manual DevTools invocation path (no auto-open)
- [ ] Step 11: Harden diagnostics, error handling, and experimental-platform behavior
- [ ] Step 12: Finalize regression suite, release gates, and support matrix verification

## Step 1: Create extension skeleton and debugger adapter registrations
Objective:
- Establish a Rust-native Zed debugger extension foundation with adapter declarations and buildable project structure.

Implementation guidance:
- Create extension manifest, Rust crate scaffolding, and extension entrypoint.
- Register debug adapters in `extension.toml` and add schema file placeholders.
- Add baseline logging category and extension initialization wiring.

Test requirements:
- Add unit test ensuring manifest/schema files are discoverable and adapter names match expected constants.
- Add build/test job that compiles the extension crate with strict warnings.

Integration notes:
- Integrate immediately with Zed dev-extension install flow to confirm extension loads without runtime panics.

Demo description:
- Install as dev extension in Zed and verify registered adapter IDs appear as available debug adapter options.

## Step 2: Define adapter schemas and configuration validation boundaries
Objective:
- Establish strict, user-facing config contracts for Dart/Flutter launch/attach/test scenarios.

Implementation guidance:
- Author JSON schemas for each adapter family and required/optional fields.
- Encode common fields and adapter-specific fields with clear constraints.
- Ensure schema defaults align with intended template behavior.

Test requirements:
- Add schema validation tests for valid and invalid config fixtures.
- Add negative tests for missing required fields and malformed attach inputs.

Integration notes:
- Wire schemas into adapter registration so Zed applies them to debug configuration editing.

Demo description:
- Open `.zed/debug.json`, validate that invalid scenario fields are rejected and valid scenarios pass.

## Step 3: Implement request-kind resolution and normalized target classification
Objective:
- Convert raw scenario config into a stable internal target-kind model and reliable launch/attach determination.

Implementation guidance:
- Implement `dap_request_kind` with explicit error on ambiguous or unsupported configurations.
- Implement target classifier for Dart launch/attach/test and Flutter launch/attach/test.
- Keep classification logic independent and testable.

Test requirements:
- Add table-driven unit tests for all target-kind variants and edge cases.
- Add tests that ensure unknown combinations fail with deterministic errors.

Integration notes:
- Connect classifier output to downstream adapter resolution API boundaries.

Demo description:
- Run scenario fixtures through the extension and show deterministic request-kind/target-kind outcomes.

## Step 4: Implement toolchain discovery and deterministic launch descriptor builder
Objective:
- Produce stable adapter startup descriptors with robust binary discovery and environment handling.

Implementation guidance:
- Resolve `dart`/`flutter` using worktree `which()` and shell env precedence.
- Support configured binary overrides where provided.
- Build a launch descriptor object containing command, args, env, cwd, and request args.

Test requirements:
- Add unit tests for path resolution precedence and missing-tool failure behavior.
- Add descriptor snapshot tests for each target-kind.

Integration notes:
- Integrate launch descriptor into `get_dap_binary` return path for all scenarios.

Demo description:
- Execute test fixtures showing generated `DebugAdapterBinary` payloads for each scenario type.

## Step 5: Deliver Dart launch and attach end-to-end sessions
Objective:
- Enable stable Dart CLI debug launch/attach using official `dart debug_adapter`.

Implementation guidance:
- Implement Dart-specific adapter argument wiring for launch and attach.
- Ensure attach scenarios properly pass VM service connection details.
- Add structured logs around adapter startup and early exit.

Test requirements:
- Add integration tests against sample Dart CLI app for breakpoint/step/evaluate/call-stack/debug-console.
- Add attach integration test using controlled VM service endpoint.

Integration notes:
- Keep session lifecycle adapter-per-session and verify clean start/stop behavior.

Demo description:
- Start a Dart CLI debug session in Zed, hit breakpoints, and complete stepping/inspect flows.

## Step 6: Deliver Dart test debugging via official test adapter mode
Objective:
- Support Dart test debugging with official `--test` adapter behavior.

Implementation guidance:
- Add test-mode mapping to `dart debug_adapter --test`.
- Ensure scenario config supports typical test entry patterns without manual edits.
- Preserve debug-console and breakpoint functionality in test runs.

Test requirements:
- Add integration tests for single-test and multi-test file flows.
- Add regression tests for watch expressions and call stack in test context.

Integration notes:
- Integrate with default templates so common Dart test projects work immediately.

Demo description:
- Launch a Dart test debug template and break inside a test case.

## Step 7: Deliver Flutter launch and attach end-to-end sessions
Objective:
- Enable stable Flutter app debug launch/attach via official `flutter debug-adapter`.

Implementation guidance:
- Implement Flutter launch/attach mapping and required config translation.
- Handle device/mode parameters through adapter config contract.
- Ensure debugger-core features remain consistent with Dart flows.

Test requirements:
- Add macOS integration tests for Flutter app launch flow with breakpoint/step/variables.
- Add attach-path tests where VM service info is provided.

Integration notes:
- Integrate Flutter-specific path into same session lifecycle and logging model.

Demo description:
- Run Flutter app debug session in Zed and validate core debugging interactions.

## Step 8: Deliver Flutter test debugging via official test adapter mode
Objective:
- Support Flutter test debugging through `flutter debug-adapter --test`.

Implementation guidance:
- Implement test-mode configuration translation for Flutter tests.
- Reuse common launch builder while applying Flutter test-specific arguments.
- Ensure stable behavior across typical widget/unit test structures.

Test requirements:
- Add integration tests for Flutter test files with breakpoints and stepping.
- Add regression test for repeated start/stop cycles in test mode.

Integration notes:
- Integrate Flutter test template into defaults and ensure no conflict with app templates.

Demo description:
- Debug a Flutter test and verify breakpoint stops and variable inspection.

## Step 9: Add New Debug Session conversion and template defaults
Objective:
- Provide strong out-of-box experience so default templates work for most projects.

Implementation guidance:
- Implement `dap_config_to_scenario` for high-level config conversion.
- Define default scenario templates for Dart app, Flutter app, Dart test, Flutter test, and attach.
- Keep template set minimal but high-value and predictable.

Test requirements:
- Add conversion tests from generic config to low-level scenario outputs.
- Add template usability smoke tests on representative fixture projects.

Integration notes:
- Integrate templates into `.zed/debug.json` guidance and project onboarding docs.

Demo description:
- In a fresh project, use default templates without manual edits and start a session successfully.

## Step 10: Add manual DevTools invocation path (no auto-open)
Objective:
- Provide explicit user-triggered DevTools flow while guaranteeing non-automatic behavior.

Implementation guidance:
- Implement manual invocation pathway bound to available Zed extension UI/action capabilities.
- Use active debug session VM service context to resolve DevTools endpoint data.
- Keep fallback path that exposes actionable endpoint info via logs when host action support is limited.
- Do not open DevTools automatically during normal debug startup.

Test requirements:
- Add tests asserting no auto-open side effects.
- Add tests for manual-trigger success and no-session failure messaging.

Integration notes:
- Integrate with debug session state tracking and logging events for DevTools interactions.

Demo description:
- Start debug session; verify no DevTools opens automatically; invoke manual path and confirm endpoint is surfaced/opened per capability.

## Step 11: Harden diagnostics, error handling, and experimental-platform behavior
Objective:
- Improve debuggability and reliability, especially around startup failures and unsupported conditions.

Implementation guidance:
- Standardize diagnostic log events for config validation, tool resolution, launch, and exit.
- Add clear user-facing failure messages for missing/misconfigured SDK tools.
- Document Linux/Windows as experimental and include safe fallback behaviors.

Test requirements:
- Add failure-path tests for missing binaries, invalid URIs, and malformed configs.
- Add log-content assertions for key diagnostics.

Integration notes:
- Integrate all error pathways into a consistent session-failure reporting contract.

Demo description:
- Trigger controlled failure scenarios and show clear logs/messages with actionable technical details.

## Step 12: Finalize regression suite, release gates, and support matrix verification
Objective:
- Enforce release quality bar and compatibility policy before v1 ship.

Implementation guidance:
- Finalize macOS CI matrix for latest stable plus previous 3 stable Dart/Flutter releases.
- Add non-blocking Linux/Windows smoke jobs marked informational.
- Establish release checklist requiring zero known blocker/critical bugs.

Test requirements:
- Run full regression suite for Dart/Flutter launch/attach/test core flows.
- Require green CI on macOS matrix lanes for release tagging.

Integration notes:
- Integrate quality gates into release workflow and issue triage process.

Demo description:
- Produce release-candidate report showing passing macOS matrix, regression success, and no blocker/critical open issues.
