# `gg reorder`

Reorder commits in your stack.

```bash
gg reorder [OPTIONS]
```

## Options

- `-o, --order <ORDER>`: New order as positions/SHAs (`"3,1,2"` or `"3 1 2"`)
- `--no-tui`: Disable the interactive TUI and use a text editor instead

## Interactive TUI

When run without `--order`, `gg reorder` opens an interactive TUI where you can visually rearrange commits:

```
┌─ Reorder Stack ──────────────────────────────────┐
│  1  abc1234  feat: add login page                │
│▸ 2  def5678  fix: handle empty input     ↕       │
│  3  ghi9012  refactor: extract validator          │
│  4  jkl3456  test: add integration tests          │
└──────────────────────────────────────────────────┘
 j/k:navigate  J/K:move commit  Enter/s:confirm  q/Esc:cancel
```

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `J` / `Shift+↓` | Move commit down |
| `K` / `Shift+↑` | Move commit up |
| `Enter` / `s` | Confirm new order |
| `q` / `Esc` | Cancel (no changes) |

Position 1 is the bottom of the stack (closest to the base branch).

The TUI requires a TTY. In non-interactive environments (pipes, CI), `gg reorder` falls back to the text editor automatically. Use `--no-tui` to force the editor fallback.

## Examples

```bash
# Interactive reorder with TUI
gg reorder

# Explicit reorder by position
gg reorder --order "3,1,2"

# Use text editor instead of TUI
gg reorder --no-tui
```
