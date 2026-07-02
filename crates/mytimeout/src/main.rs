/*
 * Copyright 2026 sukawasatoru
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
use clap::Parser;
use rust_myscript::prelude::*;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};

/// Run COMMAND, kill if still running after DURATION
#[derive(Parser, Debug)]
#[command(version)]
struct Opt {
    /// Signal to send on timeout (name or number)
    #[arg(short, long, value_name = "SIGNAL", default_value_t = String::from("TERM"))]
    signal: String,

    /// Also send KILL after this duration if still running
    #[arg(short, long, value_name = "DURATION")]
    kill_after: Option<String>,

    /// Exit with the same status as COMMAND even on timeout
    #[arg(short, long)]
    preserve_status: bool,

    /// Diagnose to stderr any signal sent upon timeout
    #[arg(short, long)]
    verbose: bool,

    /// Run COMMAND in the foreground (do not create new process group)
    #[arg(short, long)]
    foreground: bool,

    /// Duration (e.g. 10, 1.5s, 2m, 1h). 0 disables timeout.
    duration: String,

    /// Command and arguments
    #[arg(required = true, trailing_var_arg = true)]
    command: Vec<String>,
}

fn main() {
    let opt = Opt::parse();

    let dur = match parse_duration(&opt.duration) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("mytimeout: invalid duration '{}': {}", opt.duration, e);
            std::process::exit(125);
        }
    };

    let kill_after = match opt.kill_after.as_deref() {
        Some(s) => match parse_duration(s) {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("mytimeout: invalid kill-after '{}': {}", s, e);
                std::process::exit(125);
            }
        },
        None => None,
    };

    let term_sig = match parse_signal(&opt.signal) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mytimeout: invalid signal '{}': {}", opt.signal, e);
            std::process::exit(125);
        }
    };

    let mut cmd = Command::new(&opt.command[0]);
    cmd.args(&opt.command[1..])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    #[cfg(unix)]
    if !opt.foreground {
        cmd.process_group(0);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("mytimeout: command not found: {}", opt.command[0]);
            std::process::exit(127);
        }
        Err(_) => {
            eprintln!("mytimeout: failed to execute: {}", opt.command[0]);
            std::process::exit(126);
        }
    };

    let pid = child.id() as libc::pid_t;
    let target = if opt.foreground { pid } else { -pid };

    let timed_out = Arc::new(AtomicBool::new(false));
    let timed_out_flag = timed_out.clone();

    let verbose = opt.verbose;
    let _timer = if dur > Duration::ZERO {
        let ka = kill_after;
        Some(thread::spawn(move || {
            thread::sleep(dur);
            send_signal(target, term_sig, verbose);
            timed_out_flag.store(true, Ordering::SeqCst);
            if let Some(kd) = ka
                && kd > Duration::ZERO
            {
                thread::sleep(kd);
                send_signal(target, libc::SIGKILL, verbose);
            }
        }))
    } else {
        None
    };

    let status = match child.wait() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("mytimeout: wait failed: {}", e);
            std::process::exit(125);
        }
    };

    let code = if timed_out.load(Ordering::SeqCst) && !opt.preserve_status {
        124
    } else {
        exit_code_from_status(&status)
    };

    std::process::exit(code);
}

fn parse_duration(s: &str) -> Fallible<Duration> {
    let s = s.trim();
    if s.is_empty() {
        bail!("empty duration");
    }
    let (num_str, unit) = if let Some(last) = s.chars().last() {
        if last.is_alphabetic() {
            (&s[..s.len() - 1], last.to_ascii_lowercase())
        } else {
            (s, 's')
        }
    } else {
        (s, 's')
    };

    let val: f64 = num_str.parse().context("not a number")?;
    if val < 0.0 {
        bail!("negative duration not allowed");
    }

    let secs = match unit {
        's' => val,
        'm' => val * 60.0,
        'h' => val * 3600.0,
        'd' => val * 86400.0,
        other => bail!("unknown duration unit: {}", other),
    };

    Ok(Duration::from_secs_f64(secs))
}

fn parse_signal(s: &str) -> Fallible<libc::c_int> {
    let t = s.trim();
    if t.is_empty() {
        bail!("empty signal");
    }
    if let Ok(n) = t.parse::<libc::c_int>()
        && n > 0
    {
        return Ok(n);
    }
    let name = t.strip_prefix("SIG").unwrap_or(t).to_ascii_uppercase();
    let sig = match name.as_str() {
        "HUP" => libc::SIGHUP,
        "INT" => libc::SIGINT,
        "QUIT" => libc::SIGQUIT,
        "ILL" => libc::SIGILL,
        "ABRT" | "IOT" => libc::SIGABRT,
        "FPE" => libc::SIGFPE,
        "KILL" => libc::SIGKILL,
        "SEGV" => libc::SIGSEGV,
        "PIPE" => libc::SIGPIPE,
        "ALRM" => libc::SIGALRM,
        "TERM" => libc::SIGTERM,
        "USR1" => libc::SIGUSR1,
        "USR2" => libc::SIGUSR2,
        "CHLD" | "CLD" => libc::SIGCHLD,
        "CONT" => libc::SIGCONT,
        "STOP" => libc::SIGSTOP,
        "TSTP" => libc::SIGTSTP,
        "TTIN" => libc::SIGTTIN,
        "TTOU" => libc::SIGTTOU,
        "BUS" => libc::SIGBUS,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        "POLL" => libc::SIGPOLL,
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        "POLL" | "IO" => libc::SIGIO,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        "IO" => libc::SIGIO,
        "PROF" => libc::SIGPROF,
        "SYS" => libc::SIGSYS,
        "TRAP" => libc::SIGTRAP,
        "URG" => libc::SIGURG,
        "VTALRM" => libc::SIGVTALRM,
        "XCPU" => libc::SIGXCPU,
        "XFSZ" => libc::SIGXFSZ,
        _ => bail!("unknown signal name: {}", t),
    };
    Ok(sig)
}

#[cfg(unix)]
fn send_signal(target: libc::pid_t, sig: libc::c_int, verbose: bool) {
    if verbose {
        eprintln!("timeout: sending signal {} to pid {}", sig, target);
    }
    unsafe {
        libc::kill(target, sig);
    }
}

#[cfg(not(unix))]
fn send_signal(_target: i32, _sig: i32, _verbose: bool) {
    // no-op on non-unix for now
}

fn exit_code_from_status(status: &ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        code
    } else {
        #[cfg(unix)]
        if let Some(sig) = status.signal() {
            return 128 + sig;
        }
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn verify_cli() {
        Opt::command().debug_assert();
    }

    #[test]
    fn duration_parse_basic() {
        assert_eq!(parse_duration("0").unwrap(), Duration::ZERO);
        assert_eq!(parse_duration("1").unwrap(), Duration::from_secs(1));
        assert_eq!(parse_duration("1s").unwrap(), Duration::from_secs(1));
        assert_eq!(parse_duration("1.5s").unwrap(), Duration::from_millis(1500));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("0.5h").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86400));
    }

    #[test]
    fn duration_parse_errors() {
        assert!(parse_duration("").is_err());
        assert!(parse_duration("-1").is_err());
        assert!(parse_duration("1x").is_err());
        assert!(parse_duration("abc").is_err());
    }

    #[test]
    fn signal_parse_basic() {
        assert_eq!(parse_signal("TERM").unwrap(), libc::SIGTERM);
        assert_eq!(parse_signal("SIGINT").unwrap(), libc::SIGINT);
        assert_eq!(parse_signal("9").unwrap(), 9);
        assert_eq!(parse_signal("KILL").unwrap(), libc::SIGKILL);
    }

    #[test]
    fn signal_parse_errors() {
        assert!(parse_signal("").is_err());
        assert!(parse_signal("FOO").is_err());
        assert!(parse_signal("0").is_err());
    }
}
