//! Shell completion scripts for `bash`, `zsh`, and `fish`.
//!
//! The argument parser in [`crate::cli`] is hand-written, so the completion
//! scripts list the options by hand as well. A unit test keeps them in sync
//! with [`crate::cli::HELP`].

/// A shell that `mm` can generate completions for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    /// Names accepted by `--completions`.
    pub const NAMES: [&'static str; 3] = ["bash", "zsh", "fish"];

    /// Parse a shell name.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Shell::Bash),
            "zsh" => Some(Shell::Zsh),
            "fish" => Some(Shell::Fish),
            _ => None,
        }
    }

    /// Return the completion script for this shell.
    pub fn script(self) -> &'static str {
        match self {
            Shell::Bash => BASH,
            Shell::Zsh => ZSH,
            Shell::Fish => FISH,
        }
    }
}

const BASH: &str = r#"# bash completion for mm
# Install: mm --completions bash > ~/.local/share/bash-completion/completions/mm
#      or: mm --completions bash > /etc/bash_completion.d/mm

_mm() {
    local cur prev
    cur="${COMP_WORDS[COMP_CWORD]}"
    prev="${COMP_WORDS[COMP_CWORD-1]}"

    case "$prev" in
        -m|--message)
            COMPREPLY=()
            return 0
            ;;
        -c|--config)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
        --completions)
            COMPREPLY=( $(compgen -W "bash zsh fish" -- "$cur") )
            return 0
            ;;
        --chat-session|--agy-session|--claude-session|--codex-session|--devin-session|--codeium-session|--freebuff-session|--codebuff-session)
            COMPREPLY=( $(compgen -f -- "$cur") )
            return 0
            ;;
    esac

    if [[ "$cur" == --config=* ]]; then
        COMPREPLY=( $(compgen -f -- "${cur#--config=}") )
        COMPREPLY=( "${COMPREPLY[@]/#/--config=}" )
        return 0
    fi

    local opts="\
-a --add -e --edit -p --print -d --dry-run \
-m --message --chat --chat-session --agy --antigravity --agy-session \
--claude --claude-session --codex --codex-session \
--devin --codeium --devin-session --codeium-session \
--freebuff --codebuff --freebuff-session --codebuff-session \
-c --config -s --staged --no-verify --verbose -n --no-edit \
--completions -h --help -v -V --version"

    COMPREPLY=( $(compgen -W "$opts" -- "$cur") )
}

complete -F _mm mm
"#;

const ZSH: &str = r#"#compdef mm

# zsh completion for mm
# Install: mkdir -p ~/.zfunc && mm --completions zsh > ~/.zfunc/_mm
# Then add `fpath+=~/.zfunc` before `compinit` in ~/.zshrc.

_mm() {
  _arguments -s -S \
    '(-a --add)'{-a,--add}'[Stage all changes, including untracked files]' \
    '(-e --edit)'{-e,--edit}'[Open the generated message in an editor]' \
    '(-p --print -d --dry-run)'{-p,--print}'[Print the message and exit without committing]' \
    '(-p --print -d --dry-run)'{-d,--dry-run}'[Alias for --print]' \
    '(-m --message)'{-m,--message}'[Merge the description with an existing message]:message:' \
    '--chat[Copy the chat title and last assistant message]' \
    '--chat-session[Use a specific session ID or transcript path]:session:_files' \
    '--agy[Copy the Antigravity last log commit message]' \
    '--antigravity[Copy the Antigravity last log commit message]' \
    '--agy-session[Use a specific Antigravity conversation]:conversation:_files' \
    '--claude[Copy the Claude Code last log commit message]' \
    '--claude-session[Use a specific Claude Code session]:session:_files' \
    '--codex[Copy the Codex last log commit message]' \
    '--codex-session[Use a specific Codex session]:session:_files' \
    '--devin[Copy the Devin / Codeium last log commit message]' \
    '--codeium[Copy the Devin / Codeium last log commit message]' \
    '--devin-session[Use a specific Devin / Codeium session]:session:_files' \
    '--codeium-session[Use a specific Devin / Codeium session]:session:_files' \
    '--freebuff[Copy the Freebuff / Codebuff last log commit message]' \
    '--codebuff[Copy the Freebuff / Codebuff last log commit message]' \
    '--freebuff-session[Use a specific Freebuff / Codebuff session]:session:_files' \
    '--codebuff-session[Use a specific Freebuff / Codebuff session]:session:_files' \
    '(-c --config)'{-c,--config}'[Use a specific commitlint config]:file:_files' \
    '(-s --staged)'{-s,--staged}'[Only use staged changes]' \
    '--no-verify[Bypass Git commit hooks]' \
    '--verbose[Print diagnostics to stderr]' \
    '(-n --no-edit)'{-n,--no-edit}'[Compatibility flag; editing is off by default]' \
    '--completions[Print a completion script]:shell:(bash zsh fish)' \
    '(-h --help)'{-h,--help}'[Show help and exit]' \
    '(-v -V --version)'{-v,-V,--version}'[Show the version and exit]'
}

if [ "$funcstack[1]" = "_mm" ]; then
  _mm "$@"
else
  compdef _mm mm
fi
"#;

const FISH: &str = r#"# fish completion for mm
# Install: mm --completions fish > ~/.config/fish/completions/mm.fish

complete -c mm -s a -l add -d 'Stage all changes, including untracked files'
complete -c mm -s e -l edit -d 'Open the generated message in an editor'
complete -c mm -s p -l print -d 'Print the message and exit without committing'
complete -c mm -s d -l dry-run -d 'Alias for --print'
complete -c mm -s m -l message -r -d 'Merge the description with an existing message'
complete -c mm -l chat -d 'Copy the chat title and last assistant message'
complete -c mm -l chat-session -r -F -d 'Use a specific session ID or transcript path'
complete -c mm -l agy -d 'Copy the Antigravity last log commit message'
complete -c mm -l antigravity -d 'Copy the Antigravity last log commit message'
complete -c mm -l agy-session -r -F -d 'Use a specific Antigravity conversation'
complete -c mm -l claude -d 'Copy the Claude Code last log commit message'
complete -c mm -l claude-session -r -F -d 'Use a specific Claude Code session'
complete -c mm -l codex -d 'Copy the Codex last log commit message'
complete -c mm -l codex-session -r -F -d 'Use a specific Codex session'
complete -c mm -l devin -d 'Copy the Devin / Codeium last log commit message'
complete -c mm -l codeium -d 'Copy the Devin / Codeium last log commit message'
complete -c mm -l devin-session -r -F -d 'Use a specific Devin / Codeium session'
complete -c mm -l codeium-session -r -F -d 'Use a specific Devin / Codeium session'
complete -c mm -l freebuff -d 'Copy the Freebuff / Codebuff last log commit message'
complete -c mm -l codebuff -d 'Copy the Freebuff / Codebuff last log commit message'
complete -c mm -l freebuff-session -r -F -d 'Use a specific Freebuff / Codebuff session'
complete -c mm -l codebuff-session -r -F -d 'Use a specific Freebuff / Codebuff session'
complete -c mm -s c -l config -r -F -d 'Use a specific commitlint config'
complete -c mm -s s -l staged -d 'Only use staged changes'
complete -c mm -l no-verify -d 'Bypass Git commit hooks'
complete -c mm -l verbose -d 'Print diagnostics to stderr'
complete -c mm -s n -l no-edit -d 'Compatibility flag; editing is off by default'
complete -c mm -l completions -x -a 'bash zsh fish' -d 'Print a completion script'
complete -c mm -s h -l help -d 'Show help and exit'
complete -c mm -s v -l version -d 'Show the version and exit'
complete -c mm -s V -l version -d 'Show the version and exit'
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::HELP;
    use std::collections::BTreeSet;

    #[test]
    fn parses_supported_shells() {
        assert_eq!(Shell::parse("bash"), Some(Shell::Bash));
        assert_eq!(Shell::parse("zsh"), Some(Shell::Zsh));
        assert_eq!(Shell::parse("fish"), Some(Shell::Fish));
        assert_eq!(Shell::parse("nushell"), None);
        assert_eq!(Shell::parse(""), None);
    }

    #[test]
    fn scripts_cover_every_documented_option() {
        let flags: BTreeSet<String> = HELP
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|token| token.starts_with("--") && token.len() > 2)
            .map(str::to_string)
            .collect();

        assert!(flags.contains("--completions"));

        for shell in [Shell::Bash, Shell::Zsh, Shell::Fish] {
            let script = shell.script();

            for flag in &flags {
                let needle = match shell {
                    Shell::Fish => format!("-l {}", flag.trim_start_matches("--")),
                    _ => flag.clone(),
                };

                assert!(
                    script.contains(needle.as_str()),
                    "{shell:?} completions are missing {flag}"
                );
            }
        }
    }

    #[test]
    fn scripts_register_with_their_shell() {
        assert!(BASH.contains("complete -F _mm mm"));
        assert!(ZSH.starts_with("#compdef mm"));
        assert!(FISH.contains("complete -c mm"));
    }
}
