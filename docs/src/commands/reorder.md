# `gg reorder` / `gg arrange`

Reorder and/or drop commits in your stack.

`gg arrange` is an alias for `gg reorder` — they share the same implementation.

```bash
gg reorder [OPTIONS]
gg arrange [OPTIONS]
```

## Options

- `-o, --order <ORDER>`: New order as positions/SHAs (`"3,1,2"` or `"3 1 2"`)
- `--no-tui`: Disable the interactive TUI and use a text editor instead

## Interactive TUI

When run without `--order`, opens an interactive TUI where you can visually rearrange and drop commits:

```
┌─ Arrange Stack ──────────────────────────────────┐
│  1  abc1234  feat: add login page                │
│▸ 2  def5678  fix: handle empty input     ↕       │
│  [DROP] 3  ghi9012  refactor: extract validator   │
│  4  jkl3456  test: add integration tests          │
└──────────────────────────────────────────────────┘
 j/k:navigate  J/K:move commit  d:drop/undrop  Enter/s:confirm  q/Esc:cancel
```

### Keyboard shortcuts

| Key | Action |
|-----|--------|
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `J` / `Shift+↓` | Move commit down |
| `K` / `Shift+↑` | Move commit up |
| `d` / `Delete` | Toggle drop mark on commit |
| `Enter` / `s` | Confirm new order |
| `q` / `Esc` | Cancel (no changes) |

Position 1 is the bottom of the stack (closest to the base branch).

Dropped commits appear in red with strikethrough and a `[DROP]` prefix. You can still move dropped commits (e.g., to undrop them later). At least one commit must remain — you cannot drop all commits.

The TUI requires a TTY. In non-interactive environments (pipes, CI), `gg reorder` falls back to the text editor automatically. Use `--no-tui` to force the editor fallback.

## Editor Fallback

When using the editor fallback (`--no-tui` or non-TTY), you can:
- **Reorder** commits by rearranging lines
- **Drop** commits by deleting their lines

At least one commit must remain.

## Examples

```bash
# Interactive reorder/drop with TUI
gg reorder
gg arrange

# Explicit reorder by position (no dropping)
gg reorder --order "3,1,2"

# Use text editor instead of TUI
gg arrange --no-tui
```
