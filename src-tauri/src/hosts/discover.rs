use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::tokenize::parse_line;
use serde::Serialize;

/// OpenSSH's READCONF_MAX_DEPTH.
const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Location {
    pub file: PathBuf,
    /// 1-based line number.
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Host {
    pub alias: String,
    /// Every `Host` line declaring this alias, in discovery order.
    pub locations: Vec<Location>,
    /// Declared in a file included from inside a `Host`/`Match` block, so
    /// whether ssh actually reads it depends on that block matching.
    pub conditional: bool,
}

impl Host {
    /// The single declaring location, or `None` when it is ambiguous and
    /// callers should fall back to the main config file.
    pub fn source(&self) -> Option<&Location> {
        match self.locations.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Something could not be read or parsed; hosts from it may be missing.
    Error,
    /// Not fatal and not an error to ssh either, but probably unintended.
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Issue {
    pub severity: Severity,
    pub file: PathBuf,
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct Discovery {
    pub hosts: Vec<Host>,
    pub issues: Vec<Issue>,
}

pub struct Context<'a> {
    /// The user's home directory; relative includes resolve under `<home>/.ssh`.
    pub home: &'a Path,
    pub env: &'a dyn Fn(&str) -> Option<String>,
}

/// Lists concrete host aliases reachable from `config`, following `Include`.
///
/// Never fails as a whole: unreadable or malformed parts are reported in
/// `Discovery::issues` while everything else is still returned.
pub fn discover(config: &Path, ctx: &Context) -> Discovery {
    let mut scan = Scan {
        ctx,
        ssh_dir: ctx.home.join(".ssh"),
        out: Discovery::default(),
        index: HashMap::new(),
        stack: Vec::new(),
    };
    scan.read_file(config, 0, false, None);
    scan.out
}

struct Scan<'a> {
    ctx: &'a Context<'a>,
    ssh_dir: PathBuf,
    out: Discovery,
    index: HashMap<String, usize>,
    /// Canonical paths of files currently being read, for cycle detection.
    stack: Vec<PathBuf>,
}

impl Scan<'_> {
    fn issue(&mut self, severity: Severity, file: &Path, line: Option<usize>, message: String) {
        self.out.issues.push(Issue {
            severity,
            file: file.to_path_buf(),
            line,
            message,
        });
    }

    fn read_file(&mut self, path: &Path, depth: usize, conditional: bool, from: Option<&Location>) {
        let (at_file, at_line) = match from {
            Some(loc) => (loc.file.clone(), Some(loc.line)),
            None => (path.to_path_buf(), None),
        };
        if depth > MAX_INCLUDE_DEPTH {
            let msg = format!(
                "Include nested deeper than {MAX_INCLUDE_DEPTH} levels; {} skipped",
                path.display()
            );
            self.issue(Severity::Error, &at_file, at_line, msg);
            return;
        }
        let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if self.stack.contains(&canonical) {
            let msg = format!("Include cycle: {} is already being read", path.display());
            self.issue(Severity::Error, &at_file, at_line, msg);
            return;
        }
        let bytes = match fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                let msg = match e.kind() {
                    io::ErrorKind::NotFound => format!("config file not found: {}", path.display()),
                    _ => format!("cannot read {}: {e}", path.display()),
                };
                self.issue(Severity::Error, &at_file, at_line, msg);
                return;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(t) => t,
            Err(e) => {
                let msg = "file is not valid UTF-8; some aliases may display incorrectly".into();
                self.issue(Severity::Warning, path, None, msg);
                String::from_utf8_lossy(e.as_bytes()).into_owned()
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

        self.stack.push(canonical);
        // Whether the block an `Include` appears in might not apply.
        let mut block_conditional = false;
        for (i, raw) in text.lines().enumerate() {
            let loc = Location {
                file: path.to_path_buf(),
                line: i + 1,
            };
            let (keyword, args) = match parse_line(raw) {
                Ok(Some(parsed)) => parsed,
                Ok(None) => continue,
                Err(e) => {
                    self.issue(Severity::Error, path, Some(loc.line), e);
                    continue;
                }
            };
            match keyword.as_str() {
                "host" => {
                    if args.is_empty() {
                        self.issue(
                            Severity::Error,
                            path,
                            Some(loc.line),
                            "Host has no patterns".into(),
                        );
                    }
                    block_conditional = !(args.len() == 1 && args[0] == "*");
                    for pattern in args.iter().filter(|p| is_concrete(p)) {
                        self.add_host(pattern, &loc, conditional);
                    }
                }
                "match" => {
                    block_conditional = !(args.len() == 1 && args[0].eq_ignore_ascii_case("all"));
                }
                "include" => {
                    if args.is_empty() {
                        self.issue(
                            Severity::Error,
                            path,
                            Some(loc.line),
                            "Include has no arguments".into(),
                        );
                    }
                    for arg in &args {
                        self.include(arg, depth + 1, conditional || block_conditional, &loc);
                    }
                }
                _ => {}
            }
        }
        self.stack.pop();
    }

    fn include(&mut self, arg: &str, depth: usize, conditional: bool, loc: &Location) {
        let expanded = match expand(arg, self.ctx) {
            Ok(p) => p,
            Err(e) => {
                self.issue(Severity::Error, &loc.file, Some(loc.line), e);
                return;
            }
        };
        let target = Path::new(&expanded);
        let target = if target.is_absolute() || target.has_root() {
            target.to_path_buf()
        } else {
            self.ssh_dir.join(target)
        };

        if !expanded.contains(['*', '?', '[']) {
            if target.is_file() {
                self.read_file(&target, depth, conditional, Some(loc));
            } else {
                let msg = format!(
                    "Include file not found (ssh ignores it too): {}",
                    target.display()
                );
                self.issue(Severity::Warning, &loc.file, Some(loc.line), msg);
            }
            return;
        }

        let pattern = target.to_string_lossy();
        let mut matches: Vec<PathBuf> = match glob::glob(&pattern) {
            Ok(paths) => paths
                .filter_map(Result::ok)
                .filter(|p| p.is_file())
                .collect(),
            Err(e) => {
                let msg = format!("invalid Include pattern {pattern}: {e}");
                self.issue(Severity::Error, &loc.file, Some(loc.line), msg);
                return;
            }
        };
        if matches.is_empty() {
            let msg = format!("Include pattern matched no files: {pattern}");
            self.issue(Severity::Warning, &loc.file, Some(loc.line), msg);
        }
        matches.sort();
        for path in matches {
            self.read_file(&path, depth, conditional, Some(loc));
        }
    }

    fn add_host(&mut self, alias: &str, loc: &Location, conditional: bool) {
        match self.index.get(alias) {
            Some(&i) => {
                let host = &mut self.out.hosts[i];
                if !host.locations.contains(loc) {
                    host.locations.push(loc.clone());
                }
                // Unconditional if any declaration is.
                host.conditional &= conditional;
            }
            None => {
                self.index.insert(alias.to_string(), self.out.hosts.len());
                self.out.hosts.push(Host {
                    alias: alias.to_string(),
                    locations: vec![loc.clone()],
                    conditional,
                });
            }
        }
    }
}

/// A pattern that names exactly one host, as opposed to a wildcard or negation.
fn is_concrete(pattern: &str) -> bool {
    !pattern.is_empty() && !pattern.starts_with('!') && !pattern.contains(['*', '?'])
}

/// Expands a leading `~`, `${VAR}` references, and the `%d`/`%%` tokens.
fn expand(arg: &str, ctx: &Context) -> Result<String, String> {
    let home = ctx.home.to_string_lossy();
    let mut s = match arg.strip_prefix('~') {
        Some("") => home.to_string(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("{home}{rest}"),
        Some(_) => return Err(format!("~user paths are not supported in Include: {arg}")),
        None => arg.to_string(),
    };

    while let Some(start) = s.find("${") {
        let end = s[start..]
            .find('}')
            .map(|e| start + e)
            .ok_or_else(|| format!("unterminated ${{ in Include: {arg}"))?;
        let name = &s[start + 2..end];
        let value = (ctx.env)(name)
            .ok_or_else(|| format!("environment variable {name} is not set (Include {arg})"))?;
        s.replace_range(start..=end, &value);
    }

    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('d') => out.push_str(&home),
            other => {
                let token = other.map(|t| format!("%{t}")).unwrap_or_else(|| "%".into());
                return Err(format!(
                    "token {token} in Include is not supported by host discovery (ssh still honors it): {arg}"
                ));
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct Fixture {
        home: TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let home = TempDir::new().unwrap();
            fs::create_dir(home.path().join(".ssh")).unwrap();
            Fixture { home }
        }

        /// Writes a file relative to `~/.ssh` and returns its path.
        fn write(&self, rel: &str, content: &str) -> PathBuf {
            let path = self.home.path().join(".ssh").join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            path
        }

        fn discover(&self, config: &Path) -> Discovery {
            let env = |name: &str| (name == "SSH_EXTRA").then(|| "extra".to_string());
            discover(
                config,
                &Context {
                    home: self.home.path(),
                    env: &env,
                },
            )
        }

        fn run(&self, config: &str) -> Discovery {
            let path = self.write("config", config);
            self.discover(&path)
        }
    }

    fn aliases(d: &Discovery) -> Vec<&str> {
        d.hosts.iter().map(|h| h.alias.as_str()).collect()
    }

    fn messages(d: &Discovery) -> Vec<(Severity, &str)> {
        d.issues
            .iter()
            .map(|i| (i.severity, i.message.as_str()))
            .collect()
    }

    #[test]
    fn lists_concrete_aliases_in_order_and_skips_patterns() {
        let f = Fixture::new();
        let d = f.run(
            "Host alpha beta\n  HostName 10.0.0.1\n\
             Host *\n  User me\n\
             Host *.example.com web?? !blocked gamma\n\
             Match host delta\n  Port 2222\n",
        );
        assert_eq!(aliases(&d), ["alpha", "beta", "gamma"]);
        assert!(d.issues.is_empty(), "{:?}", d.issues);
    }

    #[test]
    fn records_source_lines() {
        let f = Fixture::new();
        let d = f.run("# comment\n\nHost alpha\n  Port 22\nhost=beta\n");
        let config = f.home.path().join(".ssh").join("config");
        assert_eq!(
            d.hosts[0].source(),
            Some(&Location {
                file: config.clone(),
                line: 3
            })
        );
        assert_eq!(
            d.hosts[1].source(),
            Some(&Location {
                file: config,
                line: 5
            })
        );
    }

    #[test]
    fn handles_crlf_and_bom() {
        let f = Fixture::new();
        let d = f.run("\u{feff}Host alpha\r\n  User me\r\nHost beta\r\n");
        assert_eq!(aliases(&d), ["alpha", "beta"]);
        assert_eq!(d.hosts[1].source().unwrap().line, 3);
    }

    #[test]
    fn follows_relative_includes_under_ssh_dir() {
        let f = Fixture::new();
        f.write("conf.d/b.conf", "Host from-b\n");
        f.write("conf.d/a.conf", "Host from-a\nInclude nested/deep\n");
        f.write("nested/deep", "Host from-deep\n");
        let d = f.run("Include conf.d/*.conf\nHost main\n");
        assert_eq!(aliases(&d), ["from-a", "from-deep", "from-b", "main"]);
        assert!(d.issues.is_empty(), "{:?}", d.issues);
    }

    #[test]
    fn follows_tilde_absolute_env_and_percent_d_includes() {
        let f = Fixture::new();
        let tilde = f.write("t.conf", "Host via-tilde\n");
        let abs = f.write("abs.conf", "Host via-absolute\n");
        f.write("extra.conf", "Host via-env\n");
        f.write("pct.conf", "Host via-percent\n");
        assert!(tilde.exists());
        let d = f.run(&format!(
            "Include ~/.ssh/t.conf\nInclude \"{}\"\nInclude ${{SSH_EXTRA}}.conf\nInclude %d/.ssh/pct.conf\n",
            abs.display()
        ));
        assert_eq!(
            aliases(&d),
            ["via-tilde", "via-absolute", "via-env", "via-percent"]
        );
        assert!(d.issues.is_empty(), "{:?}", d.issues);
    }

    #[test]
    fn missing_includes_are_warnings_not_fatal() {
        let f = Fixture::new();
        let d = f.run("Include missing.conf\nInclude none/*.conf\nHost alpha\n");
        assert_eq!(aliases(&d), ["alpha"]);
        let m = messages(&d);
        assert_eq!(m.len(), 2);
        assert!(m.iter().all(|(s, _)| *s == Severity::Warning));
        assert_eq!(d.issues[0].line, Some(1));
    }

    #[test]
    fn unsupported_include_forms_are_errors_on_that_line_only() {
        let f = Fixture::new();
        let d = f.run("Include ${UNSET_VAR}/x\nInclude %h.conf\nInclude ~bob/x\nHost alpha\n");
        assert_eq!(aliases(&d), ["alpha"]);
        let lines: Vec<_> = d.issues.iter().map(|i| (i.severity, i.line)).collect();
        assert_eq!(
            lines,
            [
                (Severity::Error, Some(1)),
                (Severity::Error, Some(2)),
                (Severity::Error, Some(3))
            ]
        );
    }

    #[test]
    fn include_cycle_is_reported_and_hosts_kept() {
        let f = Fixture::new();
        f.write("a.conf", "Host in-a\nInclude b.conf\n");
        f.write("b.conf", "Host in-b\nInclude a.conf\n");
        let d = f.run("Include a.conf\n");
        assert_eq!(aliases(&d), ["in-a", "in-b"]);
        assert_eq!(d.issues.len(), 1);
        assert!(d.issues[0].message.contains("cycle"));
        assert_eq!(d.issues[0].file, f.home.path().join(".ssh").join("b.conf"));
    }

    #[test]
    fn include_depth_is_limited() {
        let f = Fixture::new();
        for i in 0..20 {
            f.write(
                &format!("d{i}"),
                &format!("Host h{i}\nInclude d{}\n", i + 1),
            );
        }
        f.write("d20", "Host h20\n");
        let d = f.run("Include d0\n");
        // config is depth 0, d0 is depth 1, ... d15 is depth 16.
        assert_eq!(d.hosts.len(), 16);
        assert_eq!(d.issues.len(), 1);
        assert!(d.issues[0].message.contains("deeper"));
    }

    #[test]
    fn parse_error_skips_only_that_line() {
        let f = Fixture::new();
        let d = f.run("Host alpha\nHost \"broken\nHost beta\n");
        assert_eq!(aliases(&d), ["alpha", "beta"]);
        assert_eq!(d.issues.len(), 1);
        assert_eq!(
            (d.issues[0].severity, d.issues[0].line),
            (Severity::Error, Some(2))
        );
    }

    #[test]
    fn duplicate_alias_has_ambiguous_source() {
        let f = Fixture::new();
        f.write("other", "Host alpha\n");
        let d = f.run("Host alpha\nInclude other\n");
        assert_eq!(aliases(&d), ["alpha"]);
        assert_eq!(d.hosts[0].locations.len(), 2);
        assert_eq!(d.hosts[0].source(), None);
    }

    #[test]
    fn includes_inside_conditional_blocks_mark_hosts_conditional() {
        let f = Fixture::new();
        f.write("work", "Host work-box\n");
        f.write("any", "Host any-box\n");
        f.write("all", "Host all-box\n");
        let d = f.run(
            "Host *\n  Include any\n\
             Match all\n  Include all\n\
             Match host bastion\n  Include work\n\
             Host plain\n",
        );
        let flags: Vec<_> = d
            .hosts
            .iter()
            .map(|h| (h.alias.as_str(), h.conditional))
            .collect();
        assert_eq!(
            flags,
            [
                ("any-box", false),
                ("all-box", false),
                ("work-box", true),
                ("plain", false)
            ]
        );
    }

    #[test]
    fn missing_main_config_is_an_error() {
        let f = Fixture::new();
        let d = f.discover(&f.home.path().join(".ssh").join("config"));
        assert!(d.hosts.is_empty());
        assert_eq!(d.issues.len(), 1);
        assert_eq!(d.issues[0].severity, Severity::Error);
        assert!(d.issues[0].message.contains("not found"));
    }

    #[test]
    fn unicode_and_quoted_aliases_are_kept() {
        let f = Fixture::new();
        let d = f.run("Host máy-chủ \"two words\"\n");
        assert_eq!(aliases(&d), ["máy-chủ", "two words"]);
    }

    #[test]
    fn invalid_utf8_warns_and_keeps_hosts() {
        let f = Fixture::new();
        let path = f.home.path().join(".ssh").join("config");
        fs::write(&path, b"Host alpha\n# \xff\nHost beta\n").unwrap();
        let d = f.discover(&path);
        assert_eq!(aliases(&d), ["alpha", "beta"]);
        assert_eq!(messages(&d)[0].0, Severity::Warning);
    }

    #[test]
    fn does_not_modify_config_files() {
        let f = Fixture::new();
        let content = "# keep me\nHost alpha\n    User  me   # odd spacing\n";
        let path = f.write("config", content);
        f.discover(&path);
        assert_eq!(fs::read_to_string(&path).unwrap(), content);
    }
}
