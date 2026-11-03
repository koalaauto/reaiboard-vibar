//! Log file for the packaged app.
//!
//! Started from Finder, the app has no terminal, so everything it prints would be lost. When
//! stderr is not a terminal, stdout and stderr are redirected into a pipe. A thread stamps each
//! line with the time and appends it to `~/Library/Logs/Vibar/vibar.log` (the folder Console.app
//! already shows). The rest of the code keeps using `println!` and `eprintln!`, and the vendor
//! SDK's own messages end up in the same file.
//!
//! The file holds input names, connection events and error messages. It never holds typed text.
//! It can contain shell commands from the config when one fails, so look at it before posting it.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// When the file grows past this, it is moved to `vibar.log.1` and a new one is started.
const MAX_BYTES: u64 = 1024 * 1024;

/// Output without a line break is written out in pieces of about this size, so that a program
/// printing binary data cannot make the pump hold on to memory without limit.
const MAX_LINE: usize = 64 * 1024;

/// Set once stdout and stderr really go to the log file.
static ACTIVE: AtomicBool = AtomicBool::new(false);

/// Where the log goes. `VIBAR_LOG` overrides it, which keeps tests away from the real file.
pub fn path() -> PathBuf {
    path_from(
        std::env::var_os("VIBAR_LOG"),
        &std::env::var("HOME").unwrap_or_else(|_| ".".into()),
    )
}

fn path_from(override_path: Option<std::ffi::OsString>, home: &str) -> PathBuf {
    match override_path {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => PathBuf::from(home).join("Library/Logs/Vibar/vibar.log"),
    }
}

/// True when `fd` is `/dev/null`, which is where Finder and launchd point the output of an app.
fn is_dev_null(fd: libc::c_int) -> bool {
    // SAFETY: both stat structs are zeroed values of the right type that the calls fill in.
    unsafe {
        let mut a: libc::stat = std::mem::zeroed();
        let mut b: libc::stat = std::mem::zeroed();
        libc::fstat(fd, &mut a) == 0
            && libc::stat(c"/dev/null".as_ptr(), &mut b) == 0
            && (a.st_mode & libc::S_IFMT) == libc::S_IFCHR
            && a.st_rdev == b.st_rdev
    }
}

/// Redirect stdout and stderr into the log file. Returns whether it did.
///
/// Only when nobody is listening: both streams point to `/dev/null`, which is how Finder and
/// launchd start an app, or `VIBAR_LOG` asks for a file. A terminal, or a file or pipe the caller
/// chose (`open --stderr`, `2> file`, `2>/dev/null` with stdout still on the screen), is left alone.
/// If anything fails, the standard streams stay as they were.
pub fn init() -> bool {
    if std::env::var_os("VIBAR_LOG").is_none_or(|v| v.is_empty())
        && !(is_dev_null(libc::STDOUT_FILENO) && is_dev_null(libc::STDERR_FILENO))
    {
        return false;
    }
    let Ok(sink) = Sink::open(path(), MAX_BYTES) else {
        return false;
    };
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors pipe() writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return false;
    }
    let [read_fd, write_fd] = fds;
    // SAFETY: `read_fd` is a valid pipe read end, and from here on only the logging thread uses it.
    let reader = unsafe { File::from_raw_fd(read_fd) };
    // The reader must be running before the streams point at the pipe. Otherwise a failed thread
    // start would leave output going into a pipe nobody empties, and the app would hang once it fills.
    let spawned = std::thread::Builder::new()
        .name("logfile".into())
        .spawn(move || pump(reader, sink));
    if spawned.is_err() {
        // SAFETY: the write end is still ours and unused.
        unsafe { libc::close(write_fd) };
        return false;
    }
    if !redirect_stdio(write_fd) {
        return false; // the thread sees the pipe close and ends by itself
    }
    ACTIVE.store(true, Ordering::SeqCst);
    // A crash is the line you most want in the log, and the process may be gone before the
    // logging thread has written it. Give the thread a moment after the message is printed.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);
        drain();
    }));
    true
}

/// Print the first line of the log: version, macOS and chip. It is a separate call so that a
/// second copy of the app, which exits at once, does not write into the running one's log.
pub fn announce() {
    if ACTIVE.load(Ordering::SeqCst) {
        println!("{}", banner());
    }
}

/// Call right before the process exits. The logging thread works from a pipe, so the last lines
/// are still on their way when `exit` is called, and would be lost.
pub fn drain() {
    if ACTIVE.load(Ordering::SeqCst) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Point stdout and stderr at `write_fd` and close it. On failure both go back to what they were.
fn redirect_stdio(write_fd: libc::c_int) -> bool {
    // SAFETY: every descriptor used here is valid and owned by this function. dup2 makes descriptors
    // 1 and 2 hold the pipe open, so `write_fd` itself is closed on every path.
    unsafe {
        let saved_out = libc::dup(libc::STDOUT_FILENO);
        let saved_err = libc::dup(libc::STDERR_FILENO);
        let ok = saved_out >= 0
            && saved_err >= 0
            && libc::dup2(write_fd, libc::STDOUT_FILENO) >= 0
            && libc::dup2(write_fd, libc::STDERR_FILENO) >= 0;
        if !ok {
            if saved_out >= 0 {
                libc::dup2(saved_out, libc::STDOUT_FILENO);
            }
            if saved_err >= 0 {
                libc::dup2(saved_err, libc::STDERR_FILENO);
            }
        }
        for fd in [saved_out, saved_err, write_fd] {
            if fd >= 0 {
                libc::close(fd);
            }
        }
        ok
    }
}

/// Move lines from the pipe to the file until the process ends. It must never stop early:
/// once the pipe has no reader, `println!` would panic.
fn pump(mut pipe: File, mut sink: Sink) {
    let mut buf = [0u8; 4096];
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let n = match pipe.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        pending.extend_from_slice(&buf[..n]);
        while let Some(end) = pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = pending.drain(..=end).collect();
            sink.line(String::from_utf8_lossy(&line[..end]).trim_end_matches('\r'));
        }
        if pending.len() > MAX_LINE {
            sink.line(&String::from_utf8_lossy(&pending));
            pending.clear();
        }
    }
    if !pending.is_empty() {
        sink.line(&String::from_utf8_lossy(&pending));
    }
}

fn banner() -> String {
    let macos = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    format!(
        "--- Vibar {} started, macOS {macos}, {} ---",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    )
}

/// The log file with size-based rotation. It opens the path again when someone deletes or moves
/// the file while the app runs, so lines never end up in a file nobody can see.
struct Sink {
    path: PathBuf,
    /// `None` while the file cannot be opened. Lines are dropped until it can.
    file: Option<File>,
    size: u64,
    max: u64,
}

impl Sink {
    fn open(path: PathBuf, max: u64) -> std::io::Result<Self> {
        let file = reopen(&path, max)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            size,
            max,
        })
    }

    /// Append one line with a time stamp. A line that cannot be written is dropped, because
    /// logging must never take the app down.
    fn line(&mut self, text: &str) {
        let stamped = format!("{} {text}\n", timestamp());
        self.ensure_current();
        if self.file.is_some() && self.size + stamped.len() as u64 > self.max {
            rotate(&self.path);
            self.file = reopen(&self.path, self.max).ok();
            self.size = 0;
        }
        if let Some(file) = &mut self.file {
            if file.write_all(stamped.as_bytes()).is_ok() {
                self.size += stamped.len() as u64;
            }
        }
    }

    /// Open the path again if our handle no longer is the file at that path.
    fn ensure_current(&mut self) {
        let current = match (&self.file, std::fs::metadata(&self.path)) {
            (Some(file), Ok(on_disk)) => file
                .metadata()
                .is_ok_and(|ours| ours.dev() == on_disk.dev() && ours.ino() == on_disk.ino()),
            _ => false,
        };
        if !current {
            self.file = reopen(&self.path, self.max).ok();
            self.size = self
                .file
                .as_ref()
                .and_then(|f| f.metadata().ok())
                .map_or(0, |m| m.len());
        }
    }
}

/// Open for appending, creating the folder if needed. A file left over that is already too big
/// starts a new generation.
fn reopen(path: &Path, max: u64) -> std::io::Result<File> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max) {
        rotate(path);
    }
    open_append(path)
}

/// The log may name shell commands, so only the owner can read it.
fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// `vibar.log` becomes `vibar.log.1`; the one before that is dropped.
fn rotate(path: &Path) {
    let mut older = path.as_os_str().to_owned();
    older.push(".1");
    let _ = std::fs::rename(path, older);
}

/// Local time as `2026-11-03 14:05:09.123`.
fn timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as libc::time_t;
    // SAFETY: both pointers refer to live, correctly sized values, and localtime_r keeps no state.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        now.subsec_millis()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vibar-logfile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn default_path_is_the_macos_logs_folder_and_can_be_overridden() {
        assert_eq!(
            path_from(None, "/Users/a"),
            PathBuf::from("/Users/a/Library/Logs/Vibar/vibar.log")
        );
        assert_eq!(
            path_from(Some("".into()), "/Users/a"),
            PathBuf::from("/Users/a/Library/Logs/Vibar/vibar.log")
        );
        assert_eq!(
            path_from(Some("/tmp/x.log".into()), "/Users/a"),
            PathBuf::from("/tmp/x.log")
        );
    }

    #[test]
    fn dev_null_is_recognised_and_a_regular_file_is_not() {
        use std::os::fd::AsRawFd;
        let null = File::open("/dev/null").unwrap();
        assert!(is_dev_null(null.as_raw_fd()));
        let dir = temp_dir("devnull");
        std::fs::create_dir_all(&dir).unwrap();
        let file = File::create(dir.join("out")).unwrap();
        assert!(!is_dev_null(file.as_raw_fd()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn timestamp_has_the_documented_shape() {
        let t = timestamp();
        let shape: String = t
            .chars()
            .map(|c| if c.is_ascii_digit() { '9' } else { c })
            .collect();
        assert_eq!(shape, "9999-99-99 99:99:99.999", "{t}");
    }

    #[test]
    fn lines_are_stamped_and_appended() {
        let dir = temp_dir("append");
        let path = dir.join("nested/vibar.log");
        let mut sink = Sink::open(path.clone(), MAX_BYTES).unwrap();
        sink.line("[key] enter pressed=true");
        sink.line("[key] enter pressed=false");
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with(" [key] enter pressed=true"), "{text}");
        assert!(lines[1].ends_with(" [key] enter pressed=false"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_full_file_moves_to_dot_one_and_a_new_one_starts() {
        let dir = temp_dir("rotate");
        let path = dir.join("vibar.log");
        let mut sink = Sink::open(path.clone(), 200).unwrap();
        for i in 0..20 {
            sink.line(&format!("line {i:02}"));
        }
        let current = std::fs::read_to_string(&path).unwrap();
        let older = std::fs::read_to_string(dir.join("vibar.log.1")).unwrap();
        assert!(current.len() <= 200 && older.len() <= 200);
        assert!(
            current.contains("line 19"),
            "the newest line is in the current file"
        );
        assert!(!older.contains("line 19"));
        assert!(
            !dir.join("vibar.log.2").exists(),
            "only one older generation is kept"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_oversized_file_from_the_last_run_is_rotated_on_open() {
        let dir = temp_dir("startup");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vibar.log");
        std::fs::write(&path, "x".repeat(300)).unwrap();
        let mut sink = Sink::open(path.clone(), 200).unwrap();
        sink.line("fresh");
        assert_eq!(
            std::fs::read_to_string(dir.join("vibar.log.1"))
                .unwrap()
                .len(),
            300
        );
        assert!(std::fs::read_to_string(&path).unwrap().contains("fresh"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_comes_back_after_it_was_deleted_or_moved_while_running() {
        let dir = temp_dir("vanish");
        let path = dir.join("vibar.log");
        let mut sink = Sink::open(path.clone(), MAX_BYTES).unwrap();
        sink.line("before");
        std::fs::remove_file(&path).unwrap();
        sink.line("after delete");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("after delete"));
        std::fs::rename(&path, dir.join("moved.log")).unwrap();
        sink.line("after move");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("after move") && !text.contains("before"),
            "{text}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
        sink.line("after the folder went");
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("after the folder went"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_log_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("mode");
        let path = dir.join("vibar.log");
        let _ = Sink::open(path.clone(), MAX_BYTES).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn output_without_a_line_break_is_cut_instead_of_kept_forever() {
        let dir = temp_dir("longline");
        let path = dir.join("vibar.log");
        let sink = Sink::open(path.clone(), MAX_BYTES).unwrap();
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);
        // The writer runs on its own thread, a pipe holds only 64 KiB.
        let writer = std::thread::spawn(move || {
            let mut writer = unsafe { File::from_raw_fd(w) };
            writer.write_all(&vec![b'a'; MAX_LINE * 3]).unwrap();
            writer.write_all(b"\nok\n").unwrap();
        });
        pump(unsafe { File::from_raw_fd(r) }, sink);
        writer.join().unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert!(
            lines.len() >= 4,
            "cut into pieces, got {} lines",
            lines.len()
        );
        assert!(lines.iter().all(|l| l.len() < MAX_LINE * 2));
        assert!(lines.last().unwrap().ends_with(" ok"));
        let total: usize = lines
            .iter()
            .take(lines.len() - 1)
            .map(|l| l.len() - 24)
            .sum();
        assert_eq!(total, MAX_LINE * 3, "no byte is lost");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_redirection_leaves_the_standard_streams_alone() {
        // An invalid descriptor makes dup2 fail. The function must report it and not break stdout.
        assert!(!redirect_stdio(-1));
        println!("stdout still works after a failed redirect");
    }

    #[test]
    fn the_pump_splits_lines_and_survives_odd_input() {
        let dir = temp_dir("pump");
        let path = dir.join("vibar.log");
        let sink = Sink::open(path.clone(), MAX_BYTES).unwrap();
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (r, w) = (fds[0], fds[1]);
        let mut writer = unsafe { File::from_raw_fd(w) };
        // Two lines in one write, a CRLF, invalid UTF-8, and a last line without a newline.
        writer
            .write_all(b"one\ntwo\r\nbad \xff byte\nlast")
            .unwrap();
        drop(writer);
        pump(unsafe { File::from_raw_fd(r) }, sink);
        let text = std::fs::read_to_string(&path).unwrap();
        let tails: Vec<_> = text.lines().map(|l| l[24..].to_string()).collect();
        assert_eq!(tails, ["one", "two", "bad \u{fffd} byte", "last"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
