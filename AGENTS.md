# AGENTS.md

Instructions for AI coding agents working on **Issen**. This file only
points into the project's own documentation — it doesn't duplicate content,
and no project file depends on this one. If something below is out of
date, fix the referenced doc rather than this file.

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
  — this file intentionally doesn't repeat it.
