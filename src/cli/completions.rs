//! Bash completion script for the options and city names (`source <(astroterm --bash-completions)`).

use std::io::{self, Write};

use clap::CommandFactory;

use crate::catalog::City;

use super::Arguments;

/// Completion function: options by default, city names after `-i`/`--city`. City names are matched without the
/// quoting and escaping the user may already have typed, and inserted escaped. Compatible with bash 3.2.
const COMPLETION_FUNCTION: &str = r#"_astroterm_completions() {
    local word="${COMP_WORDS[COMP_CWORD]}"
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    case "$prev" in
        -i|--city)
            local clean_word="${word//\\/}"
            clean_word="${clean_word//\"/}"
            clean_word="${clean_word//\'/}"
            COMPREPLY=()
            for city in "${ASTROTERM_CITIES[@]}"; do
                if [[ "$city" == "${clean_word}"* ]]; then
                    printf -v city_esc '%q ' "$city"
                    COMPREPLY+=("$city_esc")
                fi
            done
            ;;
        *)
            COMPREPLY=( $(compgen -W "${ASTROTERM_OPTIONS[*]}" -- "${word}") )
            ;;
    esac
}
complete -o nospace -F _astroterm_completions astroterm
"#;

/// Write the completion script: the option list, the city list, and the completion function.
pub fn write_bash_completions(out: &mut impl Write, cities: &[City]) -> io::Result<()> {
    // every short and long option
    writeln!(out, "# Bash completions for astroterm")?;
    writeln!(out, "ASTROTERM_OPTIONS=(")?;
    for argument in Arguments::command().get_arguments() {
        if let Some(short) = argument.get_short() {
            writeln!(out, "    -{short}")?;
        }
        if let Some(long) = argument.get_long() {
            writeln!(out, "    --{long}")?;
        }
    }
    writeln!(out, ")\n")?;

    // every city name, single-quoted
    writeln!(out, "ASTROTERM_CITIES=(")?;
    for city in cities {
        writeln!(out, "{}", quote_for_bash(city.name))?;
    }
    writeln!(out, ")")?;

    write!(out, "{COMPLETION_FUNCTION}")
}

/// Wrap in single quotes, escaping embedded single quotes as `'\''` (close, escaped quote, reopen).
fn quote_for_bash(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::load_embedded_cities;

    #[test]
    fn quotes_names_with_apostrophes() {
        assert_eq!(quote_for_bash("St. John's"), r"'St. John'\''s'");
        assert_eq!(quote_for_bash("Rio de Janeiro"), "'Rio de Janeiro'");
    }

    #[test]
    fn script_lists_options_and_cities() {
        let cities = load_embedded_cities().expect("embedded cities load");
        let mut script = Vec::new();
        write_bash_completions(&mut script, &cities).unwrap();
        let script = String::from_utf8(script).unwrap();

        for option in [
            "    -a\n",
            "    --latitude\n",
            "    -B\n",
            "    --bash-completions\n",
            "    -h\n",
            "    --help\n",
        ] {
            assert!(script.contains(option), "{option:?}");
        }
        assert!(script.contains("\n'Rio de Janeiro'\n"));
        assert!(script.ends_with("complete -o nospace -F _astroterm_completions astroterm\n"));
    }
}
