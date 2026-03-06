use anyhow::Result;
use nix::sys::termios::{self, SetArg, Termios};
use nix::unistd::isatty;
use std::io::Stdin;
use std::os::fd::AsFd;
use tokio::io::{AsyncRead, AsyncWrite};

pub type DynRead = Box<dyn AsyncRead + Send + Unpin>;
pub type DynWrite = Box<dyn AsyncWrite + Send + Unpin>;

pub struct StdioHandles {
    pub input: DynRead,
    pub output: DynWrite,
    pub raw_mode_guard: Option<RawStdinGuard>,
}

impl StdioHandles {
    pub fn from_process_stdio() -> Result<Self> {
        Ok(Self {
            input: Box::new(tokio::io::stdin()),
            output: Box::new(tokio::io::stdout()),
            raw_mode_guard: RawStdinGuard::stdin()?,
        })
    }

    #[cfg(test)]
    pub fn from_parts<R, W>(input: R, output: W) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        Self {
            input: Box::new(input),
            output: Box::new(output),
            raw_mode_guard: None,
        }
    }
}

pub struct RawStdinGuard {
    stdin: Stdin,
    original: Termios,
}

impl RawStdinGuard {
    pub fn stdin() -> Result<Option<Self>> {
        let stdin = std::io::stdin();
        let Some(original) = apply_raw_mode_if_tty(&stdin)? else {
            return Ok(None);
        };

        Ok(Some(Self { stdin, original }))
    }
}

impl Drop for RawStdinGuard {
    fn drop(&mut self) {
        let _ = restore_terminal_mode(&self.stdin, &self.original);
    }
}

fn apply_raw_mode_if_tty<Fd: AsFd>(fd: Fd) -> Result<Option<Termios>> {
    let fd = fd.as_fd();
    if !isatty(fd)? {
        return Ok(None);
    }

    let original = termios::tcgetattr(fd)?;
    let mut raw = original.clone();
    termios::cfmakeraw(&mut raw);
    termios::tcsetattr(fd, SetArg::TCSANOW, &raw)?;
    Ok(Some(original))
}

fn restore_terminal_mode<Fd: AsFd>(fd: Fd, original: &Termios) -> Result<()> {
    termios::tcsetattr(fd.as_fd(), SetArg::TCSANOW, original)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_raw_mode_if_tty, restore_terminal_mode};
    use anyhow::Result;
    use nix::pty::openpty;
    use nix::sys::termios::{self, LocalFlags, OutputFlags};
    use std::fs::File;

    #[test]
    fn raw_mode_is_a_noop_for_non_tty() -> Result<()> {
        let file = File::open("/dev/null")?;
        assert!(apply_raw_mode_if_tty(&file)?.is_none());
        Ok(())
    }

    #[test]
    fn raw_mode_is_applied_and_restored_for_tty() -> Result<()> {
        let pty = openpty(None, None)?;
        let original = termios::tcgetattr(&pty.slave)?;
        let saved = apply_raw_mode_if_tty(&pty.slave)?.expect("pty slave should be a tty");

        let raw = termios::tcgetattr(&pty.slave)?;
        assert_eq!(saved, original);
        assert!(!raw.local_flags.contains(LocalFlags::ICANON));
        assert!(!raw.local_flags.contains(LocalFlags::ECHO));
        assert!(!raw.output_flags.contains(OutputFlags::OPOST));

        restore_terminal_mode(&pty.slave, &saved)?;
        let restored = termios::tcgetattr(&pty.slave)?;
        assert_eq!(restored, original);
        Ok(())
    }
}
