//! `gg init` - Generate shell integration

use clap::ValueEnum;

use crate::error::Result;

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

/// Run the init command
pub fn run(shell: Shell) -> Result<()> {
    print!("{}", shell.template());
    Ok(())
}

impl Shell {
    fn template(self) -> &'static str {
        match self {
            Shell::Bash => BASH_TEMPLATE,
            Shell::Zsh => ZSH_TEMPLATE,
            Shell::Fish => FISH_TEMPLATE,
        }
    }
}

const BASH_TEMPLATE: &str = r#"# git-gud shell integration
gg() {
    local gg_cd_file
    gg_cd_file="$(mktemp "${TMPDIR:-/tmp}/gg-cd.XXXXXX")" || return

    GG_CD_FILE="$gg_cd_file" command gg "$@"
    local gg_status=$?

    if [ "$gg_status" -eq 0 ] && [ -s "$gg_cd_file" ]; then
        local gg_cd_target
        gg_cd_target="$(cat "$gg_cd_file")"
        if [ -n "$gg_cd_target" ]; then
            cd "$gg_cd_target" || gg_status=$?
        fi
    fi

    rm -f "$gg_cd_file"
    return "$gg_status"
}
"#;

const ZSH_TEMPLATE: &str = r#"# git-gud shell integration
gg() {
    local gg_cd_file
    gg_cd_file="$(mktemp "${TMPDIR:-/tmp}/gg-cd.XXXXXX")" || return

    GG_CD_FILE="$gg_cd_file" command gg "$@"
    local gg_status=$?

    if [ "$gg_status" -eq 0 ] && [ -s "$gg_cd_file" ]; then
        local gg_cd_target
        gg_cd_target="$(cat "$gg_cd_file")"
        if [ -n "$gg_cd_target" ]; then
            cd "$gg_cd_target" || gg_status=$?
        fi
    fi

    rm -f "$gg_cd_file"
    return "$gg_status"
}
"#;

const FISH_TEMPLATE: &str = r#"# git-gud shell integration
function gg
    set -l gg_tmp_dir "$TMPDIR"
    if test -z "$gg_tmp_dir"
        set gg_tmp_dir /tmp
    end

    set -l gg_cd_file (mktemp "$gg_tmp_dir/gg-cd.XXXXXX")
    if test $status -ne 0
        return
    end

    env GG_CD_FILE="$gg_cd_file" command gg $argv
    set -l gg_status $status

    if test $gg_status -eq 0; and test -s "$gg_cd_file"
        set -l gg_cd_target (cat "$gg_cd_file")
        if test -n "$gg_cd_target"
            cd "$gg_cd_target"
            set gg_status $status
        end
    end

    rm -f "$gg_cd_file"
    return $gg_status
end
"#;
