//! rrg (redripgrep): ripgrep-compatible CLI with pinyin search.
//! Interface identical to ripgrep; pattern expansion via Rime (pinyin -> Chinese).

use std::env;
use std::io;
use std::process::{Command, ExitCode};

use rime_ls::config::{load_from_file, server_config_path};
use rime_ls::rime::Rime;
use rime_ls::utils;

/// Run a closure with stderr redirected to /dev/null (suppresses librime/glog output).
#[cfg(unix)]
fn with_stderr_suppressed<R>(f: impl FnOnce() -> R) -> R {
    use std::os::unix::io::AsRawFd;
    let stderr_fd = libc::STDERR_FILENO;
    let saved = unsafe { libc::dup(stderr_fd) };
    if saved == -1 {
        return f();
    }
    let dev_null = match std::fs::File::open("/dev/null") {
        Ok(f) => f,
        Err(_) => return f(),
    };
    let null_fd = dev_null.as_raw_fd();
    let ret = unsafe {
        libc::dup2(null_fd, stderr_fd);
        let r = f();
        libc::dup2(saved, stderr_fd);
        libc::close(saved);
        r
    };
    ret
}

#[cfg(not(unix))]
fn with_stderr_suppressed<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// Ripgrep options that take a value (we skip the next arg when scanning for pattern).
const OPTIONS_TAKE_VALUE: &[&str] = &[
    "-e", "--regexp", "-f", "-A", "-B", "-C", "-g", "--glob", "-G", "--glob-file",
    "-t", "-T", "-m", "-M", "-E", "--encoding", "--engine", "--pre", "--pre-glob",
    "-z", "--path-separator", "--max-count", "--max-files", "--max-columns",
    "--context-separator", "--type-add", "--type-clear", "-D", "--pre-glob",
    "--hosts-bin", "--sort", "-j", "--threads", "--field-match-separator",
    "--replace", "--passthru",
];

fn looks_like_pinyin(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c == '\'' || c.is_ascii_digit())
}

/// Initialize Rime from rime-ls config. Returns true if initialized.
fn init_rime_from_config() -> bool {
    let config = load_from_file(server_config_path());
    let shared = utils::expand_tilde(&config.shared_data_dir);
    let user = utils::expand_tilde(&config.user_data_dir);
    let log = utils::expand_tilde(&config.log_dir);
    let (shared, user, log) = match (
        shared.to_str(),
        user.to_str(),
        log.to_str(),
    ) {
        (Some(a), Some(b), Some(c)) => (a.to_string(), b.to_string(), c.to_string()),
        _ => return false,
    };
    Rime::init(&shared, &user, &log).is_ok()
}

/// Expand a single pattern: if it looks like pinyin, add Rime candidates; else keep as-is.
fn expand_pattern(pattern: &str) -> Vec<String> {
    if !looks_like_pinyin(pattern) {
        return vec![pattern.to_string()];
    }
    if !Rime::is_initialized() && !init_rime_from_config() {
        return vec![pattern.to_string()];
    }
    let candidates = Rime::pinyin_to_candidates(pattern);
    if candidates.is_empty() {
        return vec![pattern.to_string()];
    }
    let mut out = vec![pattern.to_string()];
    for c in candidates {
        if c != pattern && !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

/// Parse argv into: pre (options before pattern), pattern (first positional or from -e), post (paths).
/// When -e is used, we collect all -e patterns and treat them as the patterns to expand.
fn parse_ripgrep_args(args: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut pre = Vec::new();
    let mut patterns = Vec::new(); // from -e or single first positional
    let mut post = Vec::new();
    let mut i = 0;

    while i < args.len() {
        let a = &args[i];
        if a == "-e" || a == "--regexp" {
            pre.push(a.clone());
            i += 1;
            if i < args.len() {
                patterns.push(args[i].clone());
                pre.push(args[i].clone());
                i += 1;
            }
            continue;
        }
        if OPTIONS_TAKE_VALUE.iter().any(|&opt| a == opt) {
            pre.push(a.clone());
            i += 1;
            if i < args.len() {
                pre.push(args[i].clone());
                i += 1;
            }
            continue;
        }
        if a.starts_with('-') {
            pre.push(a.clone());
            i += 1;
            continue;
        }
        // first positional = pattern (when we haven't seen any -e yet)
        if patterns.is_empty() {
            patterns.push(a.clone());
            i += 1;
            break;
        }
        pre.push(a.clone());
        i += 1;
    }
    while i < args.len() {
        post.push(args[i].clone());
        i += 1;
    }
    (pre, patterns, post)
}

/// Rebuild args when we had a single positional pattern: pre + -e expanded + post.
fn rebuild_args_single_positional(pre: &[String], expanded: &[String], post: &[String]) -> Vec<String> {
    let mut out: Vec<String> = pre.to_vec();
    for p in expanded {
        out.push("-e".to_string());
        out.push(p.clone());
    }
    out.extend(post.iter().cloned());
    out
}

/// Rebuild args when we use -e: replace each -e PAT with -e PAT -e c1 -e c2 ...
fn rebuild_args_e_patterns(args: &[String], expand: impl Fn(&str) -> Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if (a == "-e" || a == "--regexp") && i + 1 < args.len() {
            let pat = &args[i + 1];
            let expanded = expand(pat);
            for p in expanded {
                out.push("-e".to_string());
                out.push(p);
            }
            i += 2;
            continue;
        }
        if OPTIONS_TAKE_VALUE.iter().any(|&opt| a == opt) {
            out.push(a.clone());
            i += 1;
            if i < args.len() {
                out.push(args[i].clone());
                i += 1;
            }
            continue;
        }
        out.push(a.clone());
        i += 1;
    }
    out
}

/// Run ripgrep (binary name: rg on PATH).
fn run_ripgrep(args: &[String]) -> io::Result<std::process::ExitStatus> {
    Command::new("rg").args(args).status()
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        // No args: run ripgrep with no args (shows help)
        let status = run_ripgrep(&[])?;
        let code = status.code().unwrap_or(2).clamp(0, 255) as u8;
        return Ok(ExitCode::from(code));
    }

    let (pre, patterns, post) = parse_ripgrep_args(&args);

    let used_e = args.iter().any(|a| a == "-e" || a == "--regexp");
    let need_rime = (patterns.len() == 1 && !used_e) || used_e;
    let ripgrep_args: Vec<String> = if need_rime {
        // Suppress librime/glog stderr during pinyin expansion
        with_stderr_suppressed(|| {
            if patterns.len() == 1 && !used_e {
                let expanded = expand_pattern(&patterns[0]);
                rebuild_args_single_positional(&pre, &expanded, &post)
            } else {
                rebuild_args_e_patterns(&args, |p| expand_pattern(p))
            }
        })
    } else {
        args
    };

    let status = run_ripgrep(&ripgrep_args)?;
    let code = status.code().unwrap_or(2).clamp(0, 255) as u8;
    Ok(ExitCode::from(code))
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("rrg (redripgrep): {}", e);
            if let Some(ioerr) = e.downcast_ref::<io::Error>() {
                if ioerr.kind() == io::ErrorKind::NotFound {
                    eprintln!("rg not found on PATH. Please install ripgrep (e.g. brew install ripgrep or cargo install ripgrep).");
                }
            }
            ExitCode::from(2)
        }
    }
}
