# Shell Integration and Completions

## Shell integration

Shell integration enables parent-shell features. Today that means `gg co <stack> --wt` can move your shell into the created or reused worktree after the command succeeds.

Add the matching line to your shell config:

```bash
# Bash
eval "$(gg init bash)"

# Zsh
eval "$(gg init zsh)"

# Fish
gg init fish | source
```

Without shell integration, `gg co --wt` still creates or reuses the worktree and prints its path, but your shell stays in the original checkout.

## Shell completions

Generate completions with:

```bash
gg completions <shell>
```

Supported shells include: `bash`, `zsh`, `fish`, `elvish`, `powershell`.

## Bash

```bash
mkdir -p ~/.local/share/bash-completion/completions
gg completions bash > ~/.local/share/bash-completion/completions/gg
```

## Zsh

```bash
mkdir -p ~/.zfunc
gg completions zsh > ~/.zfunc/_gg
```

Then in `~/.zshrc`:

```bash
fpath=(~/.zfunc $fpath)
autoload -Uz compinit && compinit
```

## Fish

```bash
mkdir -p ~/.config/fish/completions
gg completions fish > ~/.config/fish/completions/gg.fish
```
