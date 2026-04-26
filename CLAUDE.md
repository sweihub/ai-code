Making **tool calls** if you have unfinished task!

# AI Code

The project is an Rust version of `claude code`: ~/claudecode/openclaudecode

The project was splitted into:
1. ai-agent: ~/rust/ai-agent, all non TUI features, reusable codes
2. ai-code: current project, all TUI/CLI stuff.

## Notices

NEVER auto git commit - only commit after user explicit request.

run all unit tests and examples before commit.

Run all unit tests and examples when you think you are done, fix any failure!

Increment the version(major.minor.build) in Cargo.toml before you commit. (feature -> +minor, fix -> +build)

Re-write all `React Ink` TUI into Rust raratui.

Localize all environment variables from prefix `CLAUDE_CODE_` to `AI_CODE_`, `ANTHROPIC_` to `AI_`.

Localize directory name `.claude` to `.ai`, file name `CLAUDE.md` to `AI.md`.

Ensure translated Rust file starts with a comment of its source TypeScript path.

YOU MUST TRANSLATE LARGE SOURCE FILE CHUNK BY CHUNK!

No `TODO`, stubs, `feature-gate` and simplified code, implement all!

Fix or suppress any build warnings, allow dead code!

Always check original typescript logics to fix the Rust issues.

Never create simplified Rust file from typescript, must completely translate.

Never blocks the TUI.

DON'T USE LOCK INSIDE SINGLE THREAD IN RUST.

Avoid using of `unsafe` in Rust.

We should always keep the original project's structure, flavor and strict logics.

Ensure we translated all related test cases.

When you are confused or in errors, go back to read original typescript, ensure the logics are correctly translated!

Don't suspect `MiniMax` model issue, it must be your own fault!

## Raw Terminal Newline Handling

termimad's `term_text()` outputs `\r\n` (CRLF), while syntect's `syntect_highlight()` outputs bare `\n`. Do NOT apply per-block CRLF conversion — it breaks code block indentation since `\n` alone moves to next line without returning cursor to column 0.

Instead, build all content into a single string, then normalize once: `replace("\r\n", "\n")` → `replace('\n', "\r\n")`. This handles mixed output uniformly before printing. The callback in `main.rs` where `\n` → `\r\n` is done for response text accumulation is the other correct use.

