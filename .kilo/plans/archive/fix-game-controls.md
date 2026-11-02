# Integrate ECC (full install) into zona_engine

## Scope
Install ECC (agents, skills, hooks, rules) as development tooling into the repo using ECC's OpenCode target, without changing game code.

## Steps
1. Clone ECC into temp: `git clone https://github.com/affaan-m/ECC.git C:\Users\russi\AppData\Local\Temp\kilo\ecc`
2. From zona_engine root run ECC installer (Windows): `powershell -File C:\Users\russi\AppData\Local\Temp\kilo\ecc\install.ps1 --profile full --target opencode`
3. Verify generated files under `.opencode/` (and any `.kilo/` mappings). Do not overwrite project-specific `AGENTS.md` without review; merge ECC rules into existing AGENTS.md manually if needed.
4. Review `git status`/`git diff` for unintended changes (especially AGENTS.md, hooks, large skill trees).
5. Run `cargo build --workspace` and `cargo test --workspace` to confirm game unaffected.

## Risks
- ECC full install adds a large number of files (agents/skills/hooks); review before commit.
- Hook runtime requires Node.js 18+.
- ECC may overwrite AGENTS.md; preserve zona_engine-specific sections.

## Validation
- `git status` shows only intended ECC files.
- `cargo test --workspace` passes.
- Kilo/OpenCode loads ECC skills/agents.

## Open questions
- Should ECC replace or append to existing `AGENTS.md`? (Recommend: append ECC rules under a separate section, keep zona_engine sections.)
