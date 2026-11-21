# Issen

<!-- Working rules for this repository, for both AI agents (Claude Code, Codex, etc.) and people.
     Rewrite the "Project-specific" section per project; keep the "Common rules" section identical across all of sahokun's repositories. -->

## Project-specific

- Project overview & features: [`README.md`](README.md)
- Tech stack, build/lint/test commands, project layout, git workflow:
  [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)
- Architecture rationale — why a given piece of code is shaped the way it
  is, one doc per area. Read the one matching whatever you're editing,
  rather than all of them:

  | Area                                          | Doc                                              |
  | ---------------------------------------------- | -------------------------------------------------- |
  | Resident process, window show/hide, custom chrome, single instance | `docs/architecture/window-lifecycle.md` |
  | Visual design: glass panel, fonts, theme, multi-monitor | `docs/architecture/ui-appearance.md` |
  | Search index targets, index updates, provider architecture | `docs/architecture/search.md` |
  | Color picker / unit converter tools           | `docs/architecture/tools.md`                    |
  | Result pinning (usage ranking) & query history | `docs/architecture/history.md`                  |
  | Plugin ABI & action vocabulary                | `docs/architecture/plugins.md`                  |
  | Global hotkey, incremental search, keyboard shortcuts | `docs/architecture/hotkey-input.md`     |
  | i18n (language switching, CJK font fallback)  | `docs/architecture/i18n.md`                     |

- Day-to-day change history: `git log` and [`CHANGELOG.md`](CHANGELOG.md)

## Common rules

### Branches and PRs

<!-- branch-model:begin -->
- The default branch is `main`. Work on a branch and open a PR into `main`.
- PRs are squash-merged. Each PR leaves a single commit on main, titled with the PR title only.
<!-- branch-model:end -->
- Write PR titles in Conventional Commits form (`feat: ...`, `fix: ...`, etc.) so the change is clear from one line.
- Merged branches are deleted automatically.

### Never put these in history or anything published

- Do not write email addresses, tokens, passwords, details of a personal environment, or links to AI agent sessions in commits, PRs, issues, or comments.
  - Work-branch commits and PRs stay on GitHub after merging, and even the owner cannot delete them. Rewriting main's history needs a force push and a request to GitHub Support. Only PR descriptions can be fixed later by editing.
- Commit with your GitHub noreply address (`<ID>+<username>@users.noreply.github.com`) as the author email.
- Do not commit `.env`, key files, or local settings (`.claude/settings.local.json`, etc.).

### Dependencies

- Only add or update to versions published at least 7 days ago. Hijacked or swapped releases are often discovered shortly after publication.
  - Dependabot version updates wait 7 days via `cooldown`. For npm, `min-release-age=7` in `.npmrc` applies the same limit to manual updates.
- Pin GitHub Actions to a full 40-character commit SHA and keep the version as a trailing comment (e.g. `actions/checkout@<SHA> # v7.0.1`). Tags can be moved.
- Treat Dependabot PRs as a signal that updates exist. Merge them once CI passes, and for major updates, after reviewing the changes.
