# Pre-Rewrite Symbol Catalog

Every unicode / powerline symbol rendered by the pre-rewrite version of
git-branch-manager (commit `2a4eab3`, the last commit before the rewrite plan
`f181c2c`). File:line references are into that tree. Comment-only characters
are excluded except where noted.

## 1. SymbolSet (`src/ui/symbols.rs`) — mode-dependent symbols

Selected by config `symbols = ascii|unicode|powerline|auto`; auto-detection
picks powerline for iTerm.app/WezTerm/kitty/Alacritty, unicode otherwise.
Cycled at runtime with `Y`.

| Field | Used for | ASCII | Unicode | Powerline |
|---|---|---|---|---|
| `checkbox_on` | row selection, checked | `[x]` | ◉ U+25C9 | U+F046 nerd check-square |
| `checkbox_off` | row selection, unchecked | `[ ]` | ◯ U+25EF | U+F096 nerd square-o |
| `cursor_prefix` | cursor row marker | `>` | ❯ U+276F | U+E0B1 powerline thin arrow |
| `arrow_up` | ahead count in A/B column | `+` | ↑ U+2191 | U+F062 nerd arrow-up |
| `arrow_down` | behind count in A/B column | `-` | ↓ U+2193 | U+F063 nerd arrow-down |
| `current_branch` | marks checked-out branch | `*` | ● U+25CF | U+E0A0 powerline branch |
| `status_merged` | Status column: merged | `+` | ✔ U+2714 | U+F126 nerd code-fork |
| `status_squash_merged` | Status column: squash-merged | `~` | ≈ U+2248 | ● U+25CF |
| `status_unmerged` | Status column: unmerged | `-` | ✘ U+2718 | U+F00D nerd x-mark |

The two powerline checkbox glyphs (U+F046 / U+F096) and the Local-column ✓
(below) are the symbols that did NOT survive into rewrite-v2; the rewrite
replaced the checkboxes with check-circle U+F058 / circle U+F111.

## 2. Hardcoded literals (mode-independent unless noted)

### Sort indicators (column headers)
- ▲ U+25B2 ascending / ▼ U+25BC descending
  - `src/ui/branch_list.rs:51`, `src/ui/remote_branch_list.rs:33`,
    `src/ui/worktree_list.rs:36`

### Ellipsis … U+2026 (falls back to `...` in ascii mode)
- Text truncation in name/path/message columns:
  `src/ui/branch_list.rs:168`, `src/ui/remote_branch_list.rs:148`,
  `src/ui/worktree_list.rs:71`, `src/ui/tag_list.rs:106`
- Pending merge-status cells `p …` / `pending …`:
  `src/ui/branch_list.rs:312,328`, `src/ui/remote_branch_list.rs:286,302`,
  `src/ui/worktree_list.rs:269,285`
- Loading toasts ("Fetching remote branches…", "Loading worktrees…",
  "Loading remote branches…"): `src/app.rs:2157,2202,2270`

### Tracking arrow → U+2192
- Branch row Remote column, `name → origin/name` form:
  `src/ui/branch_list.rs:183`
- Also in the settings footer hint `←/→ cycle   Esc close` together with
  ← U+2190: `src/ui/settings.rs:55`

### Remotes view "Local" column
- ✓ U+2713 — local tracking branch exists (`Y` in ascii mode):
  `src/ui/remote_branch_list.rs:124`
- — U+2014 — no local branch (`-` in ascii mode):
  `src/ui/remote_branch_list.rs:125`

### Em dash — U+2014 (status-bar separator)
Separates the counts segment from the keyboard hints in every view's status
bar, plus the filter indicator line:
- `src/ui/branch_list.rs:421,448,453`
- `src/ui/remote_branch_list.rs:385,425,430`
- `src/ui/tag_list.rs:155,161,166,172,177`
- `src/ui/worktree_list.rs:341,346`

### Filter menu active markers
- ◉ U+25C9 token active / ◯ U+25EF token inactive: `src/ui/filter.rs:196`
  (same glyphs as the unicode checkboxes, but hardcoded — shown in all modes)

### Help overlay
- ↓ U+2193 / ↑ U+2191 in the key list (`j/↓`, `k/↑`): `src/ui/help.rs:8-9`
- │ U+2502 box-drawing vertical as the two-column separator (rendered as
  `  │  `): `src/ui/help.rs:62`

## 3. Comment-only (never rendered)
- ✗ U+2717 in a width-calculation comment: `src/ui/worktree_list.rs:75`
- Various — em dashes and → arrows in code comments throughout `src/app.rs`,
  `src/types.rs`, `src/git/*`

## Codepoint quick reference

| Char | Codepoint | Role(s) |
|---|---|---|
| — | U+2014 | status-bar separator; Local column "no" |
| … | U+2026 | truncation; pending status; toasts |
| ← | U+2190 | settings hint |
| ↑ | U+2191 | ahead arrow (unicode); help text |
| → | U+2192 | tracking ref arrow; settings hint |
| ↓ | U+2193 | behind arrow (unicode); help text |
| ≈ | U+2248 | squash-merged (unicode) |
| │ | U+2502 | help column separator |
| ▲ | U+25B2 | sort ascending |
| ▼ | U+25BC | sort descending |
| ◉ | U+25C9 | checkbox on (unicode); filter token active |
| ● | U+25CF | current branch (unicode); squash-merged (powerline) |
| ◯ | U+25EF | checkbox off (unicode); filter token inactive |
| ✓ | U+2713 | Local column "yes" (remotes view) |
| ✔ | U+2714 | merged (unicode) |
| ✘ | U+2718 | unmerged (unicode) |
| ❯ | U+276F | cursor prefix (unicode) |
| (PL) | U+E0A0 | current branch (powerline) |
| (PL) | U+E0B1 | cursor prefix (powerline) |
| (NF) | U+F00D | unmerged (powerline) |
| (NF) | U+F046 | checkbox on (powerline) — dropped in rewrite |
| (NF) | U+F062 | ahead arrow (powerline) |
| (NF) | U+F063 | behind arrow (powerline) |
| (NF) | U+F096 | checkbox off (powerline) — dropped in rewrite |
| (NF) | U+F126 | merged (powerline) |
