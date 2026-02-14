# Shell Completions

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
