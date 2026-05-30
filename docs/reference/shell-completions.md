# Shell Completion Scripts

RockLake ships with shell completion support for `bash`, `zsh`, and `fish`
via the `completions` subcommand (added in v0.46.0).

## Installation

### Bash

```bash
rocklake completions bash >> ~/.bash_completion
# Or, for system-wide installation:
rocklake completions bash | sudo tee /etc/bash_completion.d/rocklake > /dev/null
```

Restart your shell or run `source ~/.bash_completion` to activate.

### Zsh

```zsh
mkdir -p ~/.zfunc
rocklake completions zsh > ~/.zfunc/_rocklake
# Add to ~/.zshrc if not already present:
echo 'fpath=(~/.zfunc $fpath)' >> ~/.zshrc
echo 'autoload -Uz compinit && compinit' >> ~/.zshrc
```

Restart your shell to activate.

### Fish

```fish
rocklake completions fish > ~/.config/fish/completions/rocklake.fish
```

Fish automatically loads completion files from that directory.

## Verifying

After installation, typing `rocklake <Tab>` should show all available
subcommands, and `rocklake serve --<Tab>` should show all flags for the
`serve` subcommand.

## Generating in CI

You can regenerate the scripts at build time using:

```bash
rocklake completions bash  > scripts/completions/rocklake.bash
rocklake completions zsh   > scripts/completions/_rocklake
rocklake completions fish  > scripts/completions/rocklake.fish
```
