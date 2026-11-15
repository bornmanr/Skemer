use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The parts of `pacman.conf` that change what an update plan contains.
///
/// Skemer does not parse the whole pacman configuration. It only needs the
/// repository list and the user's existing ignore rules so that a plan matches
/// what `pacman -Syu` would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PacmanConf {
    pub repositories: Vec<String>,
    pub ignore_pkgs: Vec<String>,
    pub ignore_groups: Vec<String>,
}

/// Parse `pacman.conf` text, inlining `Include` directives.
pub fn parse(text: &str) -> PacmanConf {
    let mut conf = PacmanConf::default();
    parse_into(text, None, &mut conf, 0);
    conf
}

/// Load and parse a pacman configuration file. A missing file is an error; the
/// caller decides whether to fall back to scanning the sync directory.
pub fn load(path: &Path) -> Result<PacmanConf> {
    let mut conf = PacmanConf::default();
    parse_file(path, &mut conf, 0)?;
    Ok(conf)
}

fn parse_file(path: &Path, conf: &mut PacmanConf, depth: usize) -> Result<()> {
    if depth > 16 {
        anyhow::bail!(
            "pacman.conf Include nesting is too deep at {}",
            path.display()
        );
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("read pacman configuration {}", path.display()))?;
    parse_into(&text, Some(path), conf, depth);
    Ok(())
}

fn parse_into(text: &str, base: Option<&Path>, conf: &mut PacmanConf, depth: usize) {
    let mut section: Option<String> = None;

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(name) = section_header(line) {
            let name = name.to_string();
            if !name.eq_ignore_ascii_case("options") && !conf.repositories.contains(&name) {
                conf.repositories.push(name.clone());
            }
            section = Some(name);
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        if key.eq_ignore_ascii_case("Include") {
            if let Some(path) = resolve_include(value, base) {
                // Include files are inlined; a section opened in an included
                // file stays active for the lines that follow it.
                let _ = parse_file(&path, conf, depth + 1);
            }
            continue;
        }

        if section
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case("options"))
        {
            if key.eq_ignore_ascii_case("IgnorePkg") {
                push_words(&mut conf.ignore_pkgs, value);
            } else if key.eq_ignore_ascii_case("IgnoreGroup") {
                push_words(&mut conf.ignore_groups, value);
            }
        }
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}

fn section_header(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let inner = inner.trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

fn push_words(target: &mut Vec<String>, value: &str) {
    for word in value.split_whitespace() {
        if !target.iter().any(|existing| existing == word) {
            target.push(word.to_string());
        }
    }
}

fn resolve_include(value: &str, base: Option<&Path>) -> Option<PathBuf> {
    if value.is_empty() || value.contains(['*', '?', '[']) {
        return None;
    }
    let path = PathBuf::from(value);
    if path.is_absolute() {
        return Some(path);
    }
    base.and_then(|base| base.parent())
        .map(|parent| parent.join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_repositories_and_ignores() {
        let text = "\
            [options]\n\
            HoldPkg = pacman glibc\n\
            IgnorePkg = linux linux-firmware\n\
            IgnoreGroup = kde-applications\n\
            \n\
            [core]\n\
            Include = /etc/pacman.d/mirrorlist\n\
            \n\
            [extra]\n\
            Server = https://example.invalid/$repo/os/$arch\n";
        let conf = parse(text);
        assert_eq!(conf.repositories, vec!["core", "extra"]);
        assert_eq!(conf.ignore_pkgs, vec!["linux", "linux-firmware"]);
        assert_eq!(conf.ignore_groups, vec!["kde-applications"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let text = "# [disabled]\n[core] # trailing\n#IgnorePkg = nope\n";
        let conf = parse(text);
        assert_eq!(conf.repositories, vec!["core"]);
        assert!(conf.ignore_pkgs.is_empty());
    }

    #[test]
    fn repeated_values_are_deduplicated() {
        let text = "[options]\nIgnorePkg = a b\nIgnorePkg = b c\n";
        let conf = parse(text);
        assert_eq!(conf.ignore_pkgs, vec!["a", "b", "c"]);
    }

    #[test]
    fn ignore_keys_are_case_insensitive() {
        let text = "[OPTIONS]\nignorepkg = linux\n";
        let conf = parse(text);
        assert!(conf.repositories.is_empty());
        assert_eq!(conf.ignore_pkgs, vec!["linux"]);
    }

    #[test]
    fn include_files_are_inlined() {
        let directory =
            std::env::temp_dir().join(format!("skemer-pacman-conf-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("temp directory");
        let included = directory.join("extra.conf");
        let main = directory.join("pacman.conf");

        std::fs::write(&included, "[extra]\n[options]\nIgnorePkg = b\n").expect("write include");
        std::fs::write(
            &main,
            "[options]\nIgnorePkg = a\nInclude = extra.conf\n[core]\n",
        )
        .expect("write config");

        let conf = load(&main).expect("load config");
        assert_eq!(conf.repositories, vec!["extra", "core"]);
        assert_eq!(conf.ignore_pkgs, vec!["a", "b"]);

        std::fs::remove_dir_all(&directory).ok();
    }
}
