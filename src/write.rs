//! Writing a file dupsight owns (the HTML report, the cache) without ever
//! clobbering anything else.

use std::io::{Read, Seek, Write};
use std::path::Path;

/// Write `contents` to `path`: a new file is created only if nothing is at
/// `path` (atomically); when `existing`, the file is opened without
/// following a link, checked through that same handle to start with `mark`,
/// and only then emptied and rewritten. Whatever happens to the name
/// meanwhile, the only file written to is the one at that name, and only if
/// it was one of ours.
pub fn write_over(
    path: &Path,
    contents: &[u8],
    existing: bool,
    mark: &[u8],
) -> std::io::Result<()> {
    let changed = || std::io::Error::other("it changed during the scan; nothing was written");
    let mut f = if existing {
        let mut opts = std::fs::OpenOptions::new();
        opts.read(true).write(true);
        // If the file was swapped for a link after the check, opening it
        // must fail rather than land on (and empty) whatever it points at.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.custom_flags(libc::O_NOFOLLOW);
        }
        let mut f = match opts.open(path) {
            Ok(f) => f,
            #[cfg(unix)]
            Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return Err(changed()),
            Err(e) => return Err(e),
        };
        let mut head = vec![0u8; mark.len()];
        if !f.metadata()?.is_file() || f.read_exact(&mut head).is_err() || head != mark {
            return Err(changed());
        }
        f.set_len(0)?;
        f.seek(std::io::SeekFrom::Start(0))?;
        f
    } else {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    changed()
                } else {
                    e
                }
            })?
    };
    f.write_all(contents)?;
    f.sync_all()
}
